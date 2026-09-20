# Building and Testing

## The gate

Everything runs in a container, so the result does not depend on what happens to
be installed locally.

```bash
docker/rust.sh
```

It also runs on every push and every pull request, on x86-64 and on arm64, from
`.github/workflows/ci.yml`, alongside the front-end checks below. Both architectures, because a test has failed on one
and passed on the other before now. The same workflow scans `Cargo.lock` for
known vulnerabilities; that scan is anonymous and rate limited, so it never
fails a build on its own — only on what it actually found.

That runs, in order:

| Step | |
|---|---|
| `docker/check-headers.sh` | Every source file carries its SPDX licence line |
| `ruff check` / `ruff format --check` | The four Python tools |
| `cargo fmt --all -- --check` | |
| `cargo clippy --workspace --all-targets -- -D warnings` | Warnings are errors |
| `cargo test --workspace` | 286 tests |
| `tools/extract_contract.py --check` | The contract is still what the Python tree said |
| `tools/gen_openapi.py --check` | The specification still matches the contract |
| `tools/migrate_state.py --self-test` | The state converter round-trips |

It has to be green on x86-64 and on arm64. A test has passed on one and failed
on the other before now.

For a prompt inside the same environment:

```bash
docker/rust.sh shell
```

## The front end

The interface is ExtJS, which wants a document and a browser's idea of what is
visible, so its checks run in one:

```bash
tools/ui_harness.py            # run them, exit non-zero on failure
tools/ui_harness.py --serve    # serve the same page, for looking at by hand
```

It concatenates the bundle the way `build.rs` does, serves it to whichever
headless browser is on PATH, and reads the results out of the rendered page.
Every check in `tools/ui-harness/checks.js` is there because the thing it
checks broke once: a renderer that threw on a field the interface had stopped
asking for, a poll loop that stopped repainting, a constant on an object that
did not exist.

## Building outside the container

Needs libtorrent 2.0 and its pkg-config file:

```bash
sudo apt install libtorrent-rasterbar-dev pkg-config build-essential
cargo build --release
```

Rust dependencies are pinned in `Cargo.lock` and the container build uses
`--locked`, so a rebuild cannot resolve something newer by accident.

## Running one crate's tests

```bash
cargo test -p redeluge-daemon
cargo test -p redeluge-libtorrent --test session
```

The libtorrent tests drive a real session. Every one of them runs offline: no
DHT, no local discovery, no port mapping, and a magnet with no trackers. They
run in the build container exactly as they run on a workstation.

## How the tests are organised

Decisions are pure and tested without a session, a network or a clock: what the
schedule says for an hour, whether a watched file has settled, what a line of a
block list means, which rules an import produces. Anything that needs
libtorrent is tested against a real session rather than a mock.

Three kinds of test are worth knowing about because they have each caught
something a normal test would not:

- **Contract tests.** The implementation is checked against `contract/*.json`
  rather than against itself. This is how two missing configuration keys, one
  duplicated key and one invented key were found.
- **Conformance corpora.** 73 rencode cases and 10 real wire frames, generated
  from the Python implementation while it still existed. rencode has no written
  specification; the corpus is the specification now.
- **Hostile input.** A frame that declares 4 GB, a 200 KB body that expands to
  200 MB, a structure nested deep enough to blow the stack. These run before
  authentication in the real server, so they are tested as such.

## Regenerating what is generated

```bash
python3 tools/extract_contract.py     # the contract, from the Python tree
python3 tools/gen_openapi.py          # docs/openapi.yaml, from the contract
python3 tools/gen_rencode_corpus.py   # the conformance cases
python3 tools/gen_rpc_frames.py       # the captured frames
```

The extractor now reports that the Python tree is gone and the contract is
final. The others still run: the corpora need the Python `rencode` module, and
are only rerun when the wire format changes, which it has not since 2007.

## The container image

```bash
docker compose up -d --build
```

Two stages: the Rust build, then a runtime with no toolchain and no
interpreter. 189 MB, against 416 MB for the Python image. The build asserts
that the front-end bundle actually made it into the binary, because an asset
archive that silently came out empty looks like a working build.

## Conventions

New code matches the code around it. Beyond that:

- Every source file starts with `// SPDX-License-Identifier: GPL-3.0-or-later`,
  and the gate fails if one loses it.
- Comments explain why, not what. The ones worth writing are the ones that
  record a trap: the `.order` files that work backwards, the alert that fires on
  a transition rather than on the call.
- Tests are named as sentences that say what must be true.
- Nothing is added to the contract. It is frozen; redeluge's own additions are
  declared separately and a test enforces the distinction.
