// SPDX-License-Identifier: GPL-3.0-or-later
//! Compiles the C++ shim and links it against the system libtorrent.
//!
//! libtorrent is taken from the distribution rather than vendored: Debian
//! trixie ships 2.0.11, which is the version Deluge itself is tested against.

fn main() {
    // pkg-config gives us the include paths, the library name and, importantly,
    // the -D flags libtorrent needs for ABI consistency. Getting those wrong
    // produces a build that links and then crashes at runtime.
    let libtorrent = pkg_config::Config::new()
        .atleast_version("2.0.0")
        .probe("libtorrent-rasterbar")
        .expect(
            "libtorrent-rasterbar >= 2.0 not found. \
             On Debian/Ubuntu: apt install libtorrent-rasterbar-dev pkg-config",
        );

    let mut build = cxx_build::bridge("src/bridge.rs");
    build
        .file("src/shim.cc")
        .file("src/shim_torrent.cc")
        .file("src/shim_alert.cc")
        .include("include")
        .std("c++17")
        .flag_if_supported("-Wno-unused-parameter");

    for path in &libtorrent.include_paths {
        build.include(path);
    }
    // libtorrent's headers change layout based on these, so they have to match
    // what the library was built with.
    for (key, value) in &libtorrent.defines {
        build.define(key, value.as_deref());
    }

    build.compile("redeluge_libtorrent_shim");

    println!("cargo:rerun-if-changed=src/bridge.rs");
    println!("cargo:rerun-if-changed=src/shim.cc");
    println!("cargo:rerun-if-changed=src/shim_torrent.cc");
    println!("cargo:rerun-if-changed=src/shim_alert.cc");
    println!("cargo:rerun-if-changed=include/shim.h");
}
