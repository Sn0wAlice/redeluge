// SPDX-License-Identifier: GPL-3.0-or-later
//! Reading a `.torrent`, and a magnet link, for the add dialog.
//!
//! The dialog shows the name and the file tree before anything is added, and
//! lets each file's priority be set, so the Web UI server has to understand a
//! torrent file on its own rather than asking the daemon about a torrent that
//! does not exist yet.
//!
//! The shapes here are the ones the shipped front end reads. `files_tree` in
//! particular is a nested dictionary rather than a list, because that is what
//! `OptionsPanel.walkFileTree` walks.

use std::collections::BTreeMap;
use std::path::Path;

use serde_json::{json, Map, Value as Json};

use crate::bencode;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("could not read {path}: {source}")]
    Read {
        path: String,
        source: std::io::Error,
    },
    #[error("not a torrent file: {0}")]
    Bencode(#[from] bencode::Error),
    #[error("not a torrent file: no info dictionary")]
    NoInfo,
    #[error("not a magnet link")]
    NotAMagnet,
}

/// One file inside a torrent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileEntry {
    /// The path inside the torrent, joined with `/`.
    pub path: String,
    pub length: i64,
    pub index: usize,
}

/// What the add dialog needs to know about a torrent.
#[derive(Debug, Clone)]
pub struct TorrentInfo {
    pub name: String,
    pub info_hash: String,
    pub total_size: i64,
    pub files: Vec<FileEntry>,
}

impl TorrentInfo {
    /// The JSON the front end expects from `web.get_torrent_info`.
    pub fn to_json(&self, filename: &str) -> Json {
        json!({
            "filename": filename,
            "name": self.name,
            "info_hash": self.info_hash,
            "files_tree": self.files_tree(),
        })
    }

    /// The nested tree `OptionsPanel.walkFileTree` walks.
    ///
    /// `{"contents": {name: {"type": "dir", "contents": {...}} | {"type":
    /// "file", "index": i, "length": n, "download": true}}}`.
    pub fn files_tree(&self) -> Json {
        let mut root = Map::new();
        for file in &self.files {
            insert(&mut root, &file.path, file);
        }
        json!({ "contents": Json::Object(root) })
    }
}

fn insert(into: &mut Map<String, Json>, path: &str, file: &FileEntry) {
    let (head, rest) = match path.split_once('/') {
        Some((head, rest)) => (head, Some(rest)),
        None => (path, None),
    };
    if head.is_empty() {
        return;
    }

    match rest {
        None => {
            into.insert(
                head.to_owned(),
                json!({
                    "type": "file",
                    "index": file.index,
                    "length": file.length,
                    "download": true,
                }),
            );
        }
        Some(rest) => {
            let entry = into
                .entry(head.to_owned())
                .or_insert_with(|| json!({"type": "dir", "contents": {}, "length": 0}));
            // A path that is a file at one depth and a directory at another is
            // a malformed torrent; the directory wins rather than panicking.
            if entry.get("contents").is_none() {
                *entry = json!({"type": "dir", "contents": {}, "length": 0});
            }
            if let Some(length) = entry.get("length").and_then(Json::as_i64) {
                entry["length"] = json!(length + file.length);
            }
            if let Some(Json::Object(contents)) = entry.get_mut("contents") {
                insert(contents, rest, file);
            }
        }
    }
}

/// Reads a torrent file from disk.
pub fn read(path: &Path) -> Result<TorrentInfo, Error> {
    let bytes = std::fs::read(path).map_err(|source| Error::Read {
        path: path.display().to_string(),
        source,
    })?;
    parse(&bytes)
}

/// Reads a torrent from its bytes.
pub fn parse(bytes: &[u8]) -> Result<TorrentInfo, Error> {
    let (value, span) = bencode::decode_with_span(bytes, "info")?;
    let info = value.get("info").ok_or(Error::NoInfo)?;
    let span = span.ok_or(Error::NoInfo)?;

    // The hash is over the raw bytes as they were written. Re-encoding would
    // normalise key order and produce a hash that matches no swarm.
    let digest = ring::digest::digest(
        &ring::digest::SHA1_FOR_LEGACY_USE_ONLY,
        &bytes[span.start..span.end],
    );
    let info_hash = hex::encode(digest.as_ref());

    let name = info
        .get("name")
        .and_then(bencode::Value::as_text)
        .unwrap_or_else(|| info_hash.clone());

    let mut files = Vec::new();
    match info.get("files").and_then(bencode::Value::as_list) {
        // A multi-file torrent: every path is under the torrent's name.
        Some(entries) => {
            for (index, entry) in entries.iter().enumerate() {
                let length = entry
                    .get("length")
                    .and_then(bencode::Value::as_int)
                    .unwrap_or(0);
                let parts: Vec<String> = entry
                    .get("path")
                    .and_then(bencode::Value::as_list)
                    .map(|parts| parts.iter().filter_map(bencode::Value::as_text).collect())
                    .unwrap_or_default();
                if parts.is_empty() {
                    continue;
                }
                files.push(FileEntry {
                    path: format!("{name}/{}", parts.join("/")),
                    length,
                    index,
                });
            }
        }
        // A single-file torrent: the name is the file.
        None => {
            let length = info
                .get("length")
                .and_then(bencode::Value::as_int)
                .unwrap_or(0);
            files.push(FileEntry {
                path: name.clone(),
                length,
                index: 0,
            });
        }
    }

    let total_size = files.iter().map(|file| file.length).sum();
    Ok(TorrentInfo {
        name,
        info_hash,
        total_size,
        files,
    })
}

/// What a magnet link says about itself.
///
/// A magnet has no file list: that arrives with the metadata, after the
/// torrent has been added. The dialog knows this and shows the options tab
/// instead of the files tab when there is no tree.
pub fn magnet_info(uri: &str) -> Result<Json, Error> {
    let rest = uri.strip_prefix("magnet:?").ok_or(Error::NotAMagnet)?;

    let mut info_hash = String::new();
    let mut name = String::new();
    for pair in rest.split('&') {
        let Some((key, value)) = pair.split_once('=') else {
            continue;
        };
        match key {
            "xt" => {
                if let Some(hash) = value.strip_prefix("urn:btih:") {
                    info_hash = normalise_info_hash(hash);
                } else if let Some(hash) = value.strip_prefix("urn:btmh:") {
                    // A v2 magnet. The multihash carries a prefix; Deluge keys
                    // everything on 40 hex characters, so it is truncated the
                    // way libtorrent truncates a v2 hash.
                    let hex: String = hash.chars().skip(4).take(40).collect();
                    info_hash = normalise_info_hash(&hex);
                }
            }
            "dn" => name = percent_decode(value),
            _ => {}
        }
    }

    if info_hash.is_empty() {
        return Err(Error::NotAMagnet);
    }
    if name.is_empty() {
        name = info_hash.clone();
    }

    Ok(json!({
        "name": name,
        "info_hash": info_hash,
        "files_tree": Json::Object(Map::new()),
    }))
}

/// Lower-case hex. A base32 infohash, which some magnets use, is decoded first.
fn normalise_info_hash(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.len() == 40 && trimmed.chars().all(|c| c.is_ascii_hexdigit()) {
        return trimmed.to_lowercase();
    }
    if trimmed.len() == 32 {
        if let Some(bytes) = base32_decode(trimmed) {
            return hex::encode(bytes);
        }
    }
    trimmed.to_lowercase()
}

/// RFC 4648 base32, which is what a 32-character infohash is.
fn base32_decode(input: &str) -> Option<Vec<u8>> {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";
    let mut bits = 0u32;
    let mut count = 0u32;
    let mut out = Vec::new();

    for character in input.chars() {
        let upper = character.to_ascii_uppercase() as u8;
        let index = ALPHABET.iter().position(|candidate| *candidate == upper)?;
        bits = (bits << 5) | index as u32;
        count += 5;
        if count >= 8 {
            count -= 8;
            out.push((bits >> count) as u8);
        }
    }
    Some(out)
}

/// Percent decoding, for the display name. `+` is a space, as in a query.
fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;

    while index < bytes.len() {
        match bytes[index] {
            b'%' if index + 2 < bytes.len() => {
                let hex = std::str::from_utf8(&bytes[index + 1..index + 3]).unwrap_or("");
                match u8::from_str_radix(hex, 16) {
                    Ok(byte) => {
                        out.push(byte);
                        index += 3;
                    }
                    Err(_) => {
                        out.push(b'%');
                        index += 1;
                    }
                }
            }
            b'+' => {
                out.push(b' ');
                index += 1;
            }
            byte => {
                out.push(byte);
                index += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Where uploaded and downloaded torrent files are put.
///
/// One directory under the configuration directory rather than the system
/// temporary one: the daemon may be another process with another idea of
/// `/tmp`, and these files are read back by path.
pub fn staging_dir(config_dir: &Path) -> std::path::PathBuf {
    config_dir.join("web-uploads")
}

/// Refuses a path that is not one this server wrote.
///
/// `web.get_torrent_info` and `web.add_torrents` take a path from the browser,
/// so without this an authenticated client could read any file the server can.
/// It is still an admin-only call; that is not a reason to hand out the
/// filesystem.
pub fn is_staged(config_dir: &Path, path: &Path) -> bool {
    let staging = staging_dir(config_dir);
    let Ok(staging) = staging.canonicalize() else {
        return false;
    };
    match path.canonicalize() {
        Ok(resolved) => resolved.starts_with(&staging) && resolved.is_file(),
        Err(_) => false,
    }
}

/// A name safe to write into the staging directory.
///
/// The browser supplies it, so it cannot be trusted to be a name at all.
pub fn safe_name(raw: &str) -> String {
    let cleaned: String = raw
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | ' ') {
                c
            } else {
                '_'
            }
        })
        .collect();
    let trimmed = cleaned.trim_matches(['.', ' ']).to_owned();
    if trimmed.is_empty() {
        "upload.torrent".to_owned()
    } else {
        trimmed
    }
}

/// The tree of a torrent, as a map, for tests and for anything that wants the
/// files without the JSON.
pub fn files_by_path(info: &TorrentInfo) -> BTreeMap<String, i64> {
    info.files
        .iter()
        .map(|file| (file.path.clone(), file.length))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A single-file torrent: `d4:infod6:lengthi12e4:name8:test.txtee`.
    fn single_file() -> Vec<u8> {
        b"d4:infod6:lengthi12e4:name8:test.txtee".to_vec()
    }

    /// A two-file torrent under a directory.
    ///
    /// Built rather than written out: a hand-typed bencode literal is one
    /// miscounted length away from testing the error path instead.
    fn multi_file() -> Vec<u8> {
        fn bytes(text: &str) -> String {
            format!("{}:{text}", text.len())
        }
        let one = format!("d6:lengthi10e4:pathl{}{}ee", bytes("a"), bytes("one.txt"));
        let two = format!("d6:lengthi20e4:pathl{}ee", bytes("two.txt"));
        let info = format!("d5:filesl{one}{two}e4:name{}e", bytes("bundle"));
        format!("d4:info{info}e").into_bytes()
    }

    #[test]
    fn a_single_file_torrent_reads() {
        let info = parse(&single_file()).unwrap();
        assert_eq!(info.name, "test.txt");
        assert_eq!(info.total_size, 12);
        assert_eq!(info.files.len(), 1);
        assert_eq!(info.files[0].path, "test.txt");
        assert_eq!(info.files[0].index, 0);
    }

    #[test]
    fn a_multi_file_torrent_puts_every_path_under_the_name() {
        let info = parse(&multi_file()).unwrap();
        assert_eq!(info.name, "bundle");
        assert_eq!(info.total_size, 30);

        let files = files_by_path(&info);
        assert_eq!(files.get("bundle/a/one.txt"), Some(&10));
        assert_eq!(files.get("bundle/two.txt"), Some(&20));
    }

    #[test]
    fn the_info_hash_is_the_sha1_of_the_raw_info_dictionary() {
        // Not of a re-encoding: re-encoding normalises key order, and a torrent
        // whose producer ordered its keys unusually would then hash to
        // something that matches no swarm.
        let bytes = single_file();
        let info = parse(&bytes).unwrap();

        let raw = b"d6:lengthi12e4:name8:test.txte";
        let expected = hex::encode(
            ring::digest::digest(&ring::digest::SHA1_FOR_LEGACY_USE_ONLY, raw).as_ref(),
        );
        assert_eq!(info.info_hash, expected);
        assert_eq!(info.info_hash.len(), 40);
    }

    #[test]
    fn a_file_that_is_not_a_torrent_is_an_error() {
        assert!(parse(b"not a torrent").is_err());
        assert!(matches!(parse(b"d3:cow3:mooe"), Err(Error::NoInfo)));
        assert!(parse(b"").is_err());
    }

    #[test]
    fn the_files_tree_nests_the_way_the_front_end_walks_it() {
        let info = parse(&multi_file()).unwrap();
        let tree = info.files_tree();

        let root = &tree["contents"]["bundle"];
        assert_eq!(root["type"], json!("dir"));
        assert_eq!(root["length"], json!(30));

        assert_eq!(root["contents"]["two.txt"]["type"], json!("file"));
        assert_eq!(root["contents"]["two.txt"]["length"], json!(20));
        assert_eq!(root["contents"]["two.txt"]["index"], json!(1));

        let nested = &root["contents"]["a"];
        assert_eq!(nested["type"], json!("dir"));
        assert_eq!(nested["contents"]["one.txt"]["index"], json!(0));
    }

    #[test]
    fn a_single_file_tree_has_the_file_at_the_root() {
        let info = parse(&single_file()).unwrap();
        let tree = info.files_tree();
        assert_eq!(tree["contents"]["test.txt"]["type"], json!("file"));
        assert_eq!(tree["contents"]["test.txt"]["length"], json!(12));
    }

    // ------------------------------------------------------------- magnets

    #[test]
    fn a_magnet_gives_its_hash_and_name() {
        let uri = "magnet:?xt=urn:btih:0123456789ABCDEF0123456789abcdef01234567&dn=Some+Name";
        let info = magnet_info(uri).unwrap();
        assert_eq!(
            info["info_hash"],
            json!("0123456789abcdef0123456789abcdef01234567")
        );
        assert_eq!(info["name"], json!("Some Name"));
    }

    #[test]
    fn a_percent_encoded_name_is_decoded() {
        let uri = "magnet:?xt=urn:btih:0123456789abcdef0123456789abcdef01234567&dn=a%20b%2Fc";
        assert_eq!(magnet_info(uri).unwrap()["name"], json!("a b/c"));
    }

    #[test]
    fn a_base32_infohash_becomes_hex() {
        // Some magnets still use the 32-character form.
        let uri = "magnet:?xt=urn:btih:AEBAGBAFAYDQQCIKBMGA2DQPCAIREEYU";
        let info = magnet_info(uri).unwrap();
        assert_eq!(info["info_hash"].as_str().unwrap().len(), 40);
        assert!(info["info_hash"]
            .as_str()
            .unwrap()
            .chars()
            .all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn a_magnet_with_no_name_falls_back_to_its_hash() {
        let uri = "magnet:?xt=urn:btih:0123456789abcdef0123456789abcdef01234567";
        let info = magnet_info(uri).unwrap();
        assert_eq!(info["name"], info["info_hash"]);
    }

    #[test]
    fn a_magnet_has_no_file_tree_because_it_has_no_metadata_yet() {
        let uri = "magnet:?xt=urn:btih:0123456789abcdef0123456789abcdef01234567";
        assert_eq!(magnet_info(uri).unwrap()["files_tree"], json!({}));
    }

    #[test]
    fn something_that_is_not_a_magnet_is_refused() {
        assert!(magnet_info("https://example.invalid/x.torrent").is_err());
        assert!(magnet_info("magnet:?dn=no+hash+here").is_err());
    }

    // -------------------------------------------------------------- paths

    #[test]
    fn an_uploaded_name_cannot_escape_the_staging_directory() {
        // The browser supplies this, so it is not a name until it has been
        // made into one.
        // The separators become underscores and the leading dots are
        // trimmed, so what is left cannot climb out of the directory.
        assert_eq!(safe_name("../../etc/passwd"), "_.._etc_passwd");
        assert!(!safe_name("../../etc/passwd").contains('/'));
        assert_eq!(safe_name("a/b.torrent"), "a_b.torrent");
        assert_eq!(safe_name(""), "upload.torrent");
        assert_eq!(safe_name("..."), "upload.torrent");
        assert_eq!(safe_name("ok name.torrent"), "ok name.torrent");
    }

    #[test]
    fn only_a_file_this_server_staged_may_be_read_back() {
        let dir = tempfile::tempdir().unwrap();
        let staging = staging_dir(dir.path());
        std::fs::create_dir_all(&staging).unwrap();

        let staged = staging.join("a.torrent");
        std::fs::write(&staged, single_file()).unwrap();
        assert!(is_staged(dir.path(), &staged));

        let elsewhere = dir.path().join("secret");
        std::fs::write(&elsewhere, b"x").unwrap();
        assert!(!is_staged(dir.path(), &elsewhere));
        assert!(!is_staged(dir.path(), Path::new("/etc/passwd")));
        assert!(
            !is_staged(dir.path(), &staging.join("../secret")),
            "a traversal out of the staging directory is refused"
        );
    }

    #[test]
    fn a_directory_in_the_staging_area_is_not_a_torrent_file() {
        let dir = tempfile::tempdir().unwrap();
        let staging = staging_dir(dir.path());
        std::fs::create_dir_all(staging.join("subdir")).unwrap();
        assert!(!is_staged(dir.path(), &staging.join("subdir")));
    }
}
