// SPDX-License-Identifier: GPL-3.0-or-later
//! Bundles the Web UI assets into the binary.
//!
//! The assets are the ExtJS front end, copied into this crate so that deleting
//! the Python tree later does not take the Web UI with it. They are embedded
//! rather than read from disk so the server is one file with no asset path to
//! configure and nothing to forget to ship.
//!
//! Two of the scripts are built rather than copied: `deluge-all` and
//! `ext-extensions` are directories of sources that the Python build
//! concatenates in a specific order. That order is reproduced here, `.order`
//! files included, because ExtJS classes depend on their base being defined
//! first and a wrong order is a blank page.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

/// Source directories concatenated into one script each.
const BUNDLES: &[(&str, &str)] = &[
    ("js/deluge-all", "js/deluge-all-debug.js"),
    (
        "js/extjs/ext-extensions",
        "js/extjs/ext-extensions-debug.js",
    ),
];

fn main() {
    let assets = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets");
    println!("cargo:rerun-if-changed={}", assets.display());

    let mut files: BTreeMap<String, Vec<u8>> = BTreeMap::new();

    let bundle_sources: Vec<&Path> = BUNDLES.iter().map(|(dir, _)| Path::new(*dir)).collect();
    collect(&assets, &assets, &bundle_sources, &mut files);

    for (source_dir, output) in BUNDLES {
        let dir = assets.join(source_dir);
        if !dir.is_dir() {
            panic!("missing bundle sources: {}", dir.display());
        }
        let bundled = concatenate(&dir);
        println!(
            "cargo:warning=bundled {} into {} ({} bytes)",
            source_dir,
            output,
            bundled.len()
        );
        files.insert((*output).to_owned(), bundled);
    }

    let archive = pack(&files);
    let out = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR")).join("assets.bin");
    fs::write(&out, &archive).expect("could not write the asset archive");
}

/// Collects every file except the bundle sources, which are handled separately.
fn collect(root: &Path, dir: &Path, skip: &[&Path], into: &mut BTreeMap<String, Vec<u8>>) {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(err) => panic!("could not read {}: {err}", dir.display()),
    };

    for entry in entries.flatten() {
        let path = entry.path();
        let relative = path
            .strip_prefix(root)
            .expect("paths are under the root")
            .to_path_buf();

        if skip.iter().any(|skipped| relative.starts_with(skipped)) {
            continue;
        }
        // Editor leftovers and the ordering hints are not assets.
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with('.') || name == "__pycache__" {
            continue;
        }

        if path.is_dir() {
            collect(root, &path, skip, into);
        } else {
            let bytes = fs::read(&path)
                .unwrap_or_else(|err| panic!("could not read {}: {err}", path.display()));
            into.insert(relative.to_string_lossy().replace('\\', "/"), bytes);
        }
    }
}

/// Reproduces `minify_web_js.py`'s ordering.
///
/// Files in a directory that has subdirectories are appended; files in a leaf
/// directory are prepended. Subdirectories are visited in reverse alphabetical
/// order. A `.order` file moves the names it lists to the front of their own
/// directory. The effect is that base classes land before the classes that
/// extend them.
fn concatenate(dir: &Path) -> Vec<u8> {
    let mut ordered: Vec<PathBuf> = Vec::new();
    walk_in_order(dir, &mut ordered);

    let mut out = Vec::new();
    for path in ordered {
        let bytes = fs::read(&path)
            .unwrap_or_else(|err| panic!("could not read {}: {err}", path.display()));
        out.extend_from_slice(&bytes);
        // The Python build concatenates without a separator; a file that does
        // not end in a newline would otherwise join onto the next one.
        if !out.ends_with(b"\n") {
            out.push(b'\n');
        }
    }
    out
}

fn walk_in_order(dir: &Path, scripts: &mut Vec<PathBuf>) {
    let mut directories: Vec<PathBuf> = Vec::new();
    let mut files: Vec<String> = Vec::new();

    for entry in fs::read_dir(dir)
        .expect("readable bundle directory")
        .flatten()
    {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if path.is_dir() {
            directories.push(path);
        } else if name.ends_with(".js") {
            files.push(name);
        }
    }

    files.sort();
    directories.sort();
    directories.reverse();

    if let Ok(order) = fs::read_to_string(dir.join(".order")) {
        // Applied in file order, each moved to the front, so the LAST name
        // listed ends up first. That looks backwards and is exactly what
        // minify_web_js.py does; reversing it produces a bundle of the right
        // size and the wrong order, which is the worst kind of wrong.
        for line in order.lines() {
            if let Some(wanted) = line.strip_prefix("+ ") {
                let wanted = wanted.trim();
                if let Some(position) = files.iter().position(|name| name == wanted) {
                    let name = files.remove(position);
                    files.insert(0, name);
                }
            }
        }
    }

    if directories.is_empty() {
        for name in files.iter().rev() {
            scripts.insert(0, dir.join(name));
        }
    } else {
        scripts.extend(files.iter().map(|name| dir.join(name)));
    }

    for subdirectory in directories {
        walk_in_order(&subdirectory, scripts);
    }
}

/// A length-prefixed archive: `[u32 path len][path][u32 data len][data]`...
///
/// Deliberately trivial. A tar or zip crate would be a build dependency and a
/// runtime one for something this file reads in twenty lines.
fn pack(files: &BTreeMap<String, Vec<u8>>) -> Vec<u8> {
    let mut out = Vec::new();
    for (path, bytes) in files {
        out.extend_from_slice(&(path.len() as u32).to_le_bytes());
        out.extend_from_slice(path.as_bytes());
        out.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
        out.extend_from_slice(bytes);
    }
    out
}
