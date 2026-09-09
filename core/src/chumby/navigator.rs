//! `ChumbyNavigator`: a `NavigatorBackend` decorator (wrapped around the
//! desktop navigator in `desktop/src/player.rs`).
//!
//! Intercepts the chumby player's URL extensions before they reach the real
//! backend:
//! - `exec://<command>` — shell execution whose stdout becomes the document
//!   (chumbyflashplayer extension; the part after `exec://` is a raw shell
//!   command, NOT a URL — never parse it as one). The one command we *do*
//!   crack open is the widget-cache `curl '<url>' > <file>` download, which
//!   we service in Rust instead of via a shell (see `fetch`).
//! - `http(s)://` to chumby.com hosts / localhost daemons — answered by the
//!   host's fixture corpus.
//! Everything else passes through to the wrapped backend unchanged.

use super::host::{self, HostError};
use crate::backend::navigator::{
    ErrorResponse, NavigationMethod, NavigatorBackend, OwnedFuture, Request, SuccessResponse,
};
use crate::loader::Error;
use crate::socket::{SocketAction, SocketHandle};
use async_channel::{Receiver, Sender};
use encoding_rs::Encoding;
use indexmap::IndexMap;
use std::borrow::Cow;
use std::time::Duration;
use url::{ParseError, Url};

pub struct ChumbyNavigator<T: NavigatorBackend> {
    inner: T,
}

impl<T: NavigatorBackend> ChumbyNavigator<T> {
    pub fn new(inner: T) -> Self {
        Self { inner }
    }

    /// Answer `request` from the host, or `None` to pass through.
    fn intercept(&self, url: &str) -> Option<Result<Vec<u8>, String>> {
        let host = host::host()?;
        if let Some(command) = url.strip_prefix("exec://") {
            // The Dash's AsynchronousCommand sends `escape(cmd)`; the classic
            // sends its commands raw, and none of them carries a '%'.
            let command = percent_decode(command);
            tracing::info!(target: "chumby_host", "exec:// {command:?}");
            return Some(match host.exec(&command) {
                Ok(out) => Ok(out),
                Err(e) => Err(format!("exec fixture error: {e:?}")),
            });
        }
        // Local device paths resolve against the virtual rootfs — the same
        // filesystem view the fs natives use. Three forms reach here:
        // `file://` URLs (the licenses viewer's file:////LICENSES/gpl.txt),
        // `sys://` ones (the Dash's own local-file scheme), and the
        // scheme-less absolute paths loadMovie uses for a cached widget
        // (/tmp/widgetcache/<id>?_chumby_widget_instance_index=…). A rootfs
        // miss falls through to the real navigator, so real-disk paths (the
        // controlpanel SWF at file:///usr/share/…, {FIXTURES}-expanded
        // fixture widgets) still load. loadMovie appends the widget
        // parameters as a query string; key the lookup on the path alone
        // (fetch() rewrites scheme-less response URLs to file:// so Ruffle
        // can parse the query into the loaded movie's vars).
        let local_path = local_path(url);
        if let Some(path) = local_path {
            let path = path.split('?').next().unwrap_or(path);
            if let Some(body) = host.fs().get_file(path) {
                tracing::info!(target: "chumby_host", "rootfs HIT {url}");
                return Some(Ok(body));
            }
            return None;
        }
        match host.fetch(url) {
            Some(Ok(body)) => {
                tracing::info!(target: "chumby_host", "http fixture HIT {url}");
                Some(Ok(body))
            }
            Some(Err(HostError::NotFound(_))) => {
                // Missing fixture for a chumby host: report a clean HTTP
                // failure (the panel handles onLoad(false) everywhere) rather
                // than letting the request escape to the real chumby.com.
                Some(Err(format!("missing chumby fixture for {url}")))
            }
            Some(Err(e)) => Some(Err(format!("chumby fixture error: {e:?}"))),
            None => None,
        }
    }
}

/// The rootfs path a URL names, or `None` if it names no local file.
/// `sys://` is the Dash's local-file scheme, used where `file://` would do:
/// a resized photo (`default_theme` `com/example/PhotoHolder.as:25`), the
/// widget cache directory (`util/CacheManager.as:132`, `sys:////`) and a USB
/// photo (`settings/usbphotos/USBPhotoPanelItemInfo.as:22`). The panel
/// strips the scheme itself before `_unlink` (`ThemeCallbacks.as:196`), so
/// only loads arrive here. Multi-slash forms are normalized downstream by
/// `RootFs::resolve`.
fn local_path(url: &str) -> Option<&str> {
    url.strip_prefix("file://")
        .or_else(|| url.strip_prefix("sys://"))
        .or_else(|| url.starts_with('/').then_some(url))
}

/// `%XX` escapes to bytes, anything else untouched (AS2 `escape` output).
fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        let hex = |c: u8| (c as char).to_digit(16).map(|v| v as u8);
        match (b[i], b.get(i + 1).and_then(|&c| hex(c)), b.get(i + 2).and_then(|&c| hex(c))) {
            (b'%', Some(h), Some(l)) => {
                out.push(h << 4 | l);
                i += 3;
            }
            (c, _, _) => {
                out.push(c);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Parse the panel's widget-cache download command out of an `exec://` URL:
/// `exec://[nice -n N ]curl '<url>' > <dest>[; echo $?]`. Returns the SWF
/// URL and the cache destination path, or `None` if this isn't that command
/// (the `widgetcache` dest keeps it from matching other `exec://` traffic).
fn parse_widget_curl(url: &str) -> Option<(String, String)> {
    let cmd = url.strip_prefix("exec://")?;
    let after_curl = &cmd[cmd.find("curl ")? + "curl ".len()..];
    let q1 = after_curl.find('\'')? + 1;
    let q2 = after_curl[q1..].find('\'')? + q1;
    let fetch_url = after_curl[q1..q2].to_owned();
    let dest = cmd[cmd.find('>')? + 1..]
        .split(';')
        .next()?
        .trim()
        .to_owned();
    if dest.is_empty() || !dest.contains("widgetcache") {
        return None;
    }
    Some((fetch_url, dest))
}

/// True for the chumby hosts that carry the using surface (mirrors
/// `fixture::is_using_host`); the widget SWF download must target one.
fn is_using_url_host(url: &str) -> bool {
    let rest = url
        .strip_prefix("http://")
        .or_else(|| url.strip_prefix("https://"))
        .unwrap_or(url);
    let host = rest.split(['/', ':']).next().unwrap_or("");
    host == "xml.chumby.com" || host == "widgets.chumby.com"
}

impl<T: NavigatorBackend> NavigatorBackend for ChumbyNavigator<T> {
    fn navigate_to_url(
        &self,
        url: &str,
        target: &str,
        vars_method: Option<(NavigationMethod, IndexMap<String, String>)>,
    ) {
        // getURL/fscommand-adjacent navigation: log and pass through; the
        // panel never navigates externally on the kept screens.
        tracing::info!(target: "chumby_host", "navigate_to_url {url:?} target {target:?}");
        self.inner.navigate_to_url(url, target, vars_method)
    }

    fn fetch(&self, request: Request) -> OwnedFuture<Box<dyn SuccessResponse>, ErrorResponse> {
        // Widget-cache download: to cache a remote widget the panel shells
        // out `curl '<url>' > /…/widgetcache/<id>; echo $?` (WidgetCache,
        // canCache is hardwired on for ironforge). We ship no shell, so
        // fetch the SWF in Rust via the real backend and write it into the
        // virtual rootfs — the panel then md5-verifies and loads the cached
        // file over file://. Reimplement device touchpoints in Rust, not by
        // shelling out (NFR2). Gated like the rest of the using surface:
        // owner flag + real serial, and only for our using hosts (NFR6). The
        // echoed body is "0\n"/"1\n", the `echo $?` the panel reads back.
        if let Some((fetch_url, dest)) = parse_widget_curl(request.url()) {
            let allowed = host::host()
                .map(|h| {
                    h.config().access_chumby_com
                        && super::real_ident::has_wire_identity(h.config())
                })
                .unwrap_or(false)
                && is_using_url_host(&fetch_url);
            if allowed {
                let echo_url = request.url().to_owned();
                let sub = self.inner.fetch(Request::get(fetch_url.clone()));
                return Box::pin(async move {
                    let bytes = match sub.await {
                        Ok(resp) => resp.body().await.ok(),
                        Err(e) => {
                            tracing::warn!(target: "chumby_host",
                                "widget cache fetch {fetch_url} failed: {:?}", e.error);
                            None
                        }
                    };
                    let echo: &[u8] = match bytes {
                        Some(b) => match host::host().map(|h| h.fs().put_file(&dest, &b)) {
                            Some(Ok(())) => {
                                tracing::info!(target: "chumby_host",
                                    "widget cache download: {} bytes -> {dest}", b.len());
                                b"0\n"
                            }
                            other => {
                                tracing::warn!(target: "chumby_host",
                                    "widget cache write {dest} failed: {other:?}");
                                b"1\n"
                            }
                        },
                        None => b"1\n",
                    };
                    Ok(Box::new(BytesResponse {
                        url: echo_url,
                        body: Some(echo.to_vec()),
                    }) as Box<dyn SuccessResponse>)
                });
            }
        }
        match self.intercept(request.url()) {
            Some(Ok(body)) => {
                // A scheme-less answer must gain a parseable absolute URL:
                // SwfMovie::append_parameters_from_url silently drops the
                // query — the widget parameters, _chumby_clock_format among
                // them — when Url::parse fails on the response URL.
                let url = request.url();
                let url = if url.starts_with('/') {
                    format!("file://{url}")
                } else {
                    url.to_owned()
                };
                let response = BytesResponse {
                    url,
                    body: Some(body),
                };
                Box::pin(async move { Ok(Box::new(response) as Box<dyn SuccessResponse>) })
            }
            Some(Err(message)) => {
                let url = request.url().to_owned();
                Box::pin(async move {
                    Err(ErrorResponse {
                        url,
                        error: Error::FetchError(message),
                    })
                })
            }
            None => self.inner.fetch(request),
        }
    }

    fn resolve_url(&self, url: &str) -> Result<Url, ParseError> {
        // exec:// "URLs" are raw shell text and only flow through fetch(),
        // which receives the unresolved request string (loader's form path).
        // resolve_url is only consulted for movie loads, which never use
        // exec://, so a best-effort parse is fine here.
        if url.starts_with("exec://") {
            return Url::parse(url);
        }
        self.inner.resolve_url(url)
    }

    fn spawn_future(&mut self, future: OwnedFuture<(), Error>) {
        self.inner.spawn_future(future)
    }

    fn pre_process_url(&self, url: Url) -> Url {
        self.inner.pre_process_url(url)
    }

    fn connect_socket(
        &mut self,
        host: String,
        port: u16,
        timeout: Duration,
        handle: SocketHandle,
        receiver: Receiver<Vec<u8>>,
        sender: Sender<SocketAction>,
    ) {
        self.inner
            .connect_socket(host, port, timeout, handle, receiver, sender)
    }
}

/// In-memory response for fixture bodies.
struct BytesResponse {
    url: String,
    body: Option<Vec<u8>>,
}

impl SuccessResponse for BytesResponse {
    fn url(&self) -> Cow<'_, str> {
        Cow::Borrowed(&self.url)
    }

    fn set_url(&mut self, url: String) {
        self.url = url;
    }

    fn body(self: Box<Self>) -> OwnedFuture<Vec<u8>, Error> {
        let body = self.body.unwrap_or_default();
        Box::pin(async move { Ok(body) })
    }

    fn text_encoding(&self) -> Option<&'static Encoding> {
        None
    }

    fn status(&self) -> u16 {
        200
    }

    fn redirected(&self) -> bool {
        false
    }

    fn next_chunk(&mut self) -> OwnedFuture<Option<Vec<u8>>, Error> {
        let chunk = self.body.take();
        Box::pin(async move { Ok(chunk) })
    }

    fn expected_length(&self) -> Result<Option<u64>, Error> {
        Ok(self.body.as_ref().map(|b| b.len() as u64))
    }
}

#[cfg(test)]
mod tests {
    use super::{is_using_url_host, local_path, parse_widget_curl, percent_decode};

    #[test]
    fn local_schemes_map_to_rootfs_paths() {
        assert_eq!(local_path("file:////psp/theme.swf"), Some("//psp/theme.swf"));
        assert_eq!(local_path("sys:///tmp/photo.jpg"), Some("/tmp/photo.jpg"));
        assert_eq!(local_path("sys:////tmp/widgetcache/"), Some("//tmp/widgetcache/"));
        assert_eq!(local_path("/tmp/widgetcache/abc?x=1"), Some("/tmp/widgetcache/abc?x=1"));
        assert_eq!(local_path("http://xml.chumby.com/xml/chumbies"), None);
        assert_eq!(local_path("exec://list_mounts"), None);
    }

    #[test]
    fn decodes_as2_escape_output() {
        assert_eq!(percent_decode("cat%20%2Fproc%2F1%2Fstat%20%7C%20cut%20%2Dd%20%22%20%22%20%2Df%2023"),
                   "cat /proc/1/stat | cut -d \" \" -f 23");
        assert_eq!(percent_decode("plain -n 10 curl 'http://x/?id=AB%40CD'"), "plain -n 10 curl 'http://x/?id=AB@CD'");
        assert_eq!(percent_decode("100%"), "100%");
    }

    #[test]
    fn parses_widget_cache_curl_command() {
        let url = "exec://nice -n 10 curl 'http://xml.chumby.com/xml/movie_files?id=AB%40CD' \
                   > /tmp/widgetcache/32d53436-c9b5-ba1a-639b-ead60a734f4a; echo $?";
        let (fetch, dest) = parse_widget_curl(url).expect("should parse");
        assert_eq!(fetch, "http://xml.chumby.com/xml/movie_files?id=AB%40CD");
        assert_eq!(dest, "/tmp/widgetcache/32d53436-c9b5-ba1a-639b-ead60a734f4a");
        assert!(is_using_url_host(&fetch));
    }

    #[test]
    fn ignores_non_widget_execs() {
        // Not a curl-to-widgetcache command.
        assert!(parse_widget_curl("exec://mkdir /tmp/widgetcache; sync; echo $?").is_none());
        assert!(parse_widget_curl("exec://curl 'http://x/y' > /tmp/other; echo $?").is_none());
        assert!(parse_widget_curl("http://xml.chumby.com/xml/chumbies").is_none());
        assert!(!is_using_url_host("http://update.chumby.com/update"));
    }
}
