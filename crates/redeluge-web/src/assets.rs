// SPDX-License-Identifier: GPL-3.0-or-later
//! The Web UI assets, embedded at build time.
//!
//! `build.rs` packs `assets/` into one archive and concatenates the two script
//! bundles. Serving from memory means the binary is self-contained: there is no
//! asset directory to configure, and deleting the Python tree cannot break the
//! Web UI.

use std::collections::HashMap;
use std::sync::OnceLock;

/// The packed archive. One file, so compiling it is fast.
const ARCHIVE: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/assets.bin"));

/// Every embedded file, keyed by its path relative to the web root.
pub fn files() -> &'static HashMap<&'static str, &'static [u8]> {
    static FILES: OnceLock<HashMap<&'static str, &'static [u8]>> = OnceLock::new();
    FILES.get_or_init(|| unpack(ARCHIVE))
}

pub fn get(path: &str) -> Option<&'static [u8]> {
    files().get(path.trim_start_matches('/')).copied()
}

pub fn contains(path: &str) -> bool {
    get(path).is_some()
}

fn unpack(mut archive: &'static [u8]) -> HashMap<&'static str, &'static [u8]> {
    let mut files = HashMap::new();

    while archive.len() >= 4 {
        let (path, rest) = take_block(archive);
        let (data, rest) = take_block(rest);
        let path = std::str::from_utf8(path).expect("asset paths are UTF-8");
        files.insert(path, data);
        archive = rest;
    }
    files
}

fn take_block(input: &'static [u8]) -> (&'static [u8], &'static [u8]) {
    let length = u32::from_le_bytes([input[0], input[1], input[2], input[3]]) as usize;
    let body = &input[4..4 + length];
    (body, &input[4 + length..])
}

/// The media type for a path, from its extension.
///
/// Getting this wrong is not cosmetic: a stylesheet served as `text/plain` is
/// ignored by the browser, and the page renders unstyled.
pub fn content_type(path: &str) -> &'static str {
    match path.rsplit('.').next().unwrap_or("") {
        "html" => "text/html; charset=utf-8",
        "js" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" => "application/json",
        "png" => "image/png",
        "gif" => "image/gif",
        "jpg" | "jpeg" => "image/jpeg",
        "svg" => "image/svg+xml",
        "ico" => "image/vnd.microsoft.icon",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        "txt" => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}
