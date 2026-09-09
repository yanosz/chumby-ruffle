//! The Dash's theme catalog, and the file commands its picker issues.
//!
//! The picker (`settings/themes/ThemesPanelLoadingThemes.as`) loads
//! `files.chumby.com/dash/<release>/themes/themes.xml` and parses it with
//! `themenetwork/ThemeCatalogItem.fromXML`; here the catalog is derived from
//! the themes directory of the virtual rootfs, so a theme dropped there is
//! offered without any server (survey §1.5). Picking one runs, through
//! `exec://`, `download_theme <url> <md5>` (`ThemesPanelLoadingTheme.as:28`,
//! the package's `download_theme` script: fetch to `/tmp/theme.swf`, check
//! the md5, answer `<download_theme error="…"/>`), then
//! `cp /tmp/theme.swf /psp/theme.swf; rm /tmp/theme.swf /tmp/theme.swf.sig; sync; echo $?`
//! (`:69`), or `rm /tmp/theme.swf; sync; echo $?` on failure (`:83`); the
//! scheduler's updater deletes with `rm /psp/theme.swf /psp/theme.swf.sig; …`
//! (`ThemesPanelUpdateTheme.as:124`). Those four shapes are interpreted on
//! the rootfs here — Rust, not a shell (NFR2) — and only `file://` themes
//! are "downloaded": nothing is fetched from chumby.com (NFR6).

use super::host::{ChumbyFs, DirEntryResult};
use super::real_ident::md5_hex;

/// Where local themes live; chumby's own `uninstall_chumby.sh` removes
/// `/psp/themes`, so the name is theirs.
pub const THEMES_DIR: &str = "/psp/themes";

/// `dash/<release>/themes/themes.xml` on files.chumby.com.
pub fn is_catalog_path(path: &str) -> bool {
    let mut parts = path.trim_matches('/').split('/');
    parts.next() == Some("dash")
        && parts.next().is_some()
        && parts.next() == Some("themes")
        && parts.next() == Some("themes.xml")
        && parts.next().is_none()
}

/// `<themes>` for every `*.swf` in the themes directory, sorted by name.
/// Thumbnail: a `.jpg` or `.png` beside the theme, if there is one.
pub fn catalog_xml(fs: &dyn ChumbyFs) -> String {
    let mut names: Vec<String> = Vec::new();
    for index in 0.. {
        match fs.dir_entry(THEMES_DIR, index) {
            DirEntryResult::Entry(e) if e.is_file && e.name.ends_with(".swf") => names.push(e.name),
            DirEntryResult::Entry(_) => {}
            _ => break,
        }
    }
    names.sort();
    let mut out = String::from("<themes>");
    for (i, file) in names.iter().enumerate() {
        let stem = &file[..file.len() - 4];
        let path = format!("{THEMES_DIR}/{file}");
        let md5 = fs.get_file(&path).map(|b| md5_hex(&b)).unwrap_or_default();
        let thumb = ["jpg", "png"]
            .iter()
            .map(|ext| format!("{THEMES_DIR}/{stem}.{ext}"))
            .find(|p| fs.file_exists(p))
            .map(|p| format!("file:///{p}"))
            .unwrap_or_default();
        out.push_str(&format!(
            "<theme id=\"{}\" version=\"1\"><name>{}</name><description>local theme</description>\
             <author>local</author><thumbnailURL>{}</thumbnailURL><url>file:///{}</url><md5>{}</md5>\
             <layouts/></theme>",
            i + 1,
            xml_escape(&stem.replace('_', " ")),
            xml_escape(&thumb),
            xml_escape(&path),
            md5
        ));
    }
    out.push_str("</themes>\n");
    out
}

/// The picker's commands, or `None` when this is not one of them.
pub fn exec(fs: &dyn ChumbyFs, command: &str) -> Option<Vec<u8>> {
    let command = command.trim();
    if let Some(args) = command
        .strip_prefix("/psp/download_theme ")
        .or_else(|| command.strip_prefix("download_theme "))
    {
        return Some(download_theme(fs, args).into_bytes());
    }
    if !(command.starts_with("cp ") || command.starts_with("rm ")) {
        return None;
    }
    let mut status = 0;
    for step in command.split(';').map(str::trim) {
        let mut words = step.split_whitespace();
        match (words.next(), words.next(), words.next(), words.next()) {
            (Some("cp"), Some(src), Some(dst), None) => match fs.get_file(&unescape(src)) {
                Some(body) if fs.put_file(&unescape(dst), &body).is_ok() => {}
                _ => status = 1,
            },
            (Some("rm"), Some(_), _, _) => {
                for path in step.split_whitespace().skip(1) {
                    let _ = fs.unlink(&unescape(path));
                }
            }
            (Some("sync"), None, _, _) | (Some("echo"), Some("$?"), None, _) => {}
            _ => return None, // not a shape we know
        }
    }
    Some(format!("{status}\n").into_bytes())
}

/// `download_theme <url> <md5>` for a `file://` url: copy into
/// `/tmp/theme.swf` when the md5 matches. Anything else is "not found" — no
/// chumby.com traffic.
fn download_theme(fs: &dyn ChumbyFs, args: &str) -> String {
    let mut words = args.split_whitespace();
    let (Some(url), Some(md5)) = (words.next(), words.next()) else {
        return error("Syntax: download_theme <url> <md5>");
    };
    let url = unescape(url);
    let Some(path) = url.strip_prefix("file://") else {
        return error("theme not found\nPlease choose a different theme");
    };
    let Some(body) = fs.get_file(path) else {
        return error("theme not found\nPlease choose a different theme");
    };
    if !md5_hex(&body).eq_ignore_ascii_case(md5) {
        return error("checksum failed\nPlease choose a different theme");
    }
    if fs.put_file("/tmp/theme.swf", &body).is_err() {
        return error("cannot write theme");
    }
    "<download_theme error=\"success\" />\n".to_owned()
}

fn error(message: &str) -> String {
    format!("<download_theme error=\"{}\" />\n", xml_escape(message))
}

/// `AsynchronousCommand.shellEscape` backslashes shell metacharacters.
fn unescape(s: &str) -> String {
    s.replace('\\', "")
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('"', "&quot;").replace('<', "&lt;")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chumby::host::{DirEntry, HostError};
    use std::collections::BTreeMap;
    use std::sync::Mutex;

    /// Files by path; directories are implied by their contents. Paths are
    /// normalized as `RootFs::resolve` does, since the panel spells them
    /// with doubled slashes ("file:////psp/...").
    struct MemFs(Mutex<BTreeMap<String, Vec<u8>>>);

    fn norm(path: &str) -> String {
        let mut out = String::new();
        for c in path.split('/').filter(|c| !c.is_empty() && *c != ".") {
            out.push('/');
            out.push_str(c);
        }
        out
    }

    impl ChumbyFs for MemFs {
        fn get_file(&self, path: &str) -> Option<Vec<u8>> {
            self.0.lock().unwrap().get(&norm(path)).cloned()
        }
        fn put_file(&self, path: &str, data: &[u8]) -> Result<(), HostError> {
            self.0.lock().unwrap().insert(norm(path), data.to_vec());
            Ok(())
        }
        fn file_exists(&self, path: &str) -> bool {
            self.0.lock().unwrap().contains_key(&norm(path))
        }
        fn file_size(&self, path: &str) -> Option<u64> {
            self.get_file(path).map(|b| b.len() as u64)
        }
        fn unlink(&self, path: &str) -> Result<(), HostError> {
            self.0.lock().unwrap().remove(&norm(path));
            Ok(())
        }
        fn dir_entry(&self, path: &str, index: u32) -> DirEntryResult {
            let prefix = format!("{}/", norm(path));
            let names: Vec<String> = self
                .0
                .lock()
                .unwrap()
                .keys()
                .filter_map(|k| k.strip_prefix(&prefix).map(str::to_owned))
                .collect();
            match names.get(index as usize) {
                Some(name) => DirEntryResult::Entry(DirEntry {
                    name: name.clone(),
                    is_dir: false,
                    is_dir_link: false,
                    is_file: true,
                }),
                None => DirEntryResult::End,
            }
        }
    }

    fn fs() -> MemFs {
        let mut m = BTreeMap::new();
        m.insert("/psp/themes/space_theme.swf".to_owned(), b"FWS-space".to_vec());
        m.insert("/psp/themes/space_theme.png".to_owned(), b"png".to_vec());
        m.insert("/psp/themes/blue.swf".to_owned(), b"FWS-blue".to_vec());
        m.insert("/psp/themes/notes.txt".to_owned(), b"x".to_vec());
        MemFs(Mutex::new(m))
    }

    #[test]
    fn catalog_lists_the_themes_directory() {
        let fs = fs();
        let xml = catalog_xml(&fs);
        assert!(xml.starts_with("<themes><theme id=\"1\" version=\"1\"><name>blue</name>"));
        assert!(xml.contains("<url>file:////psp/themes/blue.swf</url>"));
        assert!(xml.contains("<name>space theme</name>"));
        assert!(xml.contains("<thumbnailURL>file:////psp/themes/space_theme.png</thumbnailURL>"));
        assert!(xml.contains(&format!("<md5>{}</md5>", md5_hex(b"FWS-space"))));
        assert!(!xml.contains("notes"));
        assert!(is_catalog_path("dash/production/themes/themes.xml"));
        assert!(!is_catalog_path("dash/production/controlpanel/controlpanel.xml"));
    }

    #[test]
    fn download_then_copy_installs_a_local_theme() {
        let fs = fs();
        let md5 = md5_hex(b"FWS-blue");
        let ok = exec(&fs, &format!("download_theme file:////psp/themes/blue.swf {md5}")).unwrap();
        assert_eq!(ok, b"<download_theme error=\"success\" />\n");
        assert_eq!(fs.get_file("/tmp/theme.swf").unwrap(), b"FWS-blue");
        let copied = exec(&fs, "cp /tmp/theme.swf /psp/theme.swf; rm /tmp/theme.swf /tmp/theme.swf.sig; sync; echo $?").unwrap();
        assert_eq!(copied, b"0\n");
        assert_eq!(fs.get_file("/psp/theme.swf").unwrap(), b"FWS-blue");
        assert!(!fs.file_exists("/tmp/theme.swf"));
        let bad = exec(&fs, "download_theme file:////psp/themes/blue.swf 00").unwrap();
        assert!(String::from_utf8_lossy(&bad).contains("checksum failed"));
        let remote = exec(&fs, "download_theme http://files.chumby.com/x.swf 00").unwrap();
        assert!(String::from_utf8_lossy(&remote).contains("theme not found"));
        assert_eq!(exec(&fs, "rm /psp/theme.swf /psp/theme.swf.sig; sync; echo $?").unwrap(), b"0\n");
        assert!(!fs.file_exists("/psp/theme.swf"));
        assert!(exec(&fs, "cp a b; reboot").is_none(), "unknown step: not ours");
        assert!(exec(&fs, "list_mounts").is_none());
    }
}
