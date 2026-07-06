//! `ChumbyNavigator`: a `NavigatorBackend` decorator (hook H4).
//!
//! Intercepts the chumby player's URL extensions before they reach the real
//! backend:
//! - `exec://<command>` — shell execution whose stdout becomes the document
//!   (chumbyflashplayer extension; the part after `exec://` is a raw shell
//!   command, NOT a URL — never parse it).
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
            tracing::info!(target: "chumby_host", "exec:// {command:?}");
            return Some(match host.exec(command) {
                Ok(out) => Ok(out),
                Err(e) => Err(format!("exec fixture error: {e:?}")),
            });
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
        match self.intercept(request.url()) {
            Some(Ok(body)) => {
                let response = BytesResponse {
                    url: request.url().to_owned(),
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
