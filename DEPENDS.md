# Dependencies

## To run

- **libtorrent-rasterbar 2.0** and the OpenSSL it was built against. From the
  distribution rather than vendored: Debian trixie ships 2.0.11, the version
  Deluge is tested against.

On Debian or Ubuntu:

    sudo apt install libtorrent-rasterbar2.0t64

That is the whole runtime. The two binaries are static apart from libtorrent
and libc, and the Web UI assets are compiled into `redeluge-web`.

## To build

- **Rust**, the version in `rust-toolchain.toml`.
- **A C++17 compiler**, for the libtorrent bridge.
- **libtorrent-rasterbar-dev** and **pkg-config**, which the bridge's build
  script uses to find the headers and the ABI flags.

      sudo apt install build-essential libtorrent-rasterbar-dev pkg-config

Rust dependencies are pinned in `Cargo.lock` and the container build uses
`--locked`, so a rebuild cannot resolve something newer by accident.

## To migrate from the Python Deluge

- **Python 3**, standard library only, for `tools/migrate_state.py`. Needed
  once, and not by anything that runs afterwards.

## To work on it

- **Docker**, which is how the tests run: `docker/rust.sh` builds the
  environment and runs the whole gate, so the result does not depend on what is
  installed locally.
- The tools under `tools/` that regenerate the conformance corpora need the
  Python `rencode` module. They are only rerun when the wire format changes,
  which it has not since 2007.
