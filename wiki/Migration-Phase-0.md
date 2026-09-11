# Phase 0 report: framing and the libtorrent bridge

> **Archived.** Part of the Rust migration, which is finished.
> See [Migration Overview](Migration-Overview) for the whole story, and
> the pages under **Using redeluge** for how the software works today.

**Verdict: go.** A `cxx` bridge to libtorrent 2.0 builds, links and runs on both
architectures, the alert loop works, and errors cross as values rather than
taking the process down. Nothing found here argues against continuing.

## What phase 0 had to answer

| Question | Answer |
|---|---|
| Can Rust drive libtorrent 2.0 through `cxx`? | Yes, on x86-64 and arm64 |
| Does the flattened-alert design hold? | Yes, all 24 handled alerts and the unhandled ones |
| Can the API contract be frozen from the source? | Yes, extracted and checked automatically |
| Do C++ exceptions arrive as `Result`? | Yes, verified under repeated failure |

## What was built

2 541 lines of new code, 737 of them tests.

| Component | Lines | What it does |
|---|---|---|
| `tools/extract_contract.py` | 416 | Reads the Python tree, writes the contract |
| `crates/redeluge-libtorrent/src/shim.cc` | 380 | Flattens alerts, owns the session |
| `crates/redeluge-contract/src/lib.rs` | 244 | The contract, typed |
| `crates/redeluge-libtorrent/src/session.rs` | 266 | Safe session API |
| `crates/redeluge-libtorrent/src/alert.rs` | 210 | Typed alerts with named payloads |
| `crates/redeluge-libtorrent/src/bridge.rs` | 123 | The FFI declarations |
| `crates/redeluge-libtorrent/{lib.rs,shim.h}` | 122 | Public surface and C++ header |

The contract itself is 3 085 lines of generated JSON: 99 RPC methods with their
authorisation levels and signatures, 22 events, 96 configuration keys, and the
24 libtorrent alerts the daemon handles.

## Test results

45 tests, green on both architectures, with `cargo fmt --check` and
`cargo clippy -D warnings` as part of the same gate.

| Suite | Tests | Covers |
|---|---|---|
| `redeluge-contract/tests/contract.rs` | 16 | Contract invariants, no libtorrent needed |
| `redeluge-libtorrent/tests/session.rs` | 23 | A real libtorrent session, offline |
| `redeluge-libtorrent/tests/contract.rs` | 5 | Alert enum against the contract |
| Doc tests | 1 | The published example compiles |

Every session test runs with DHT, local discovery and port mapping disabled, on
a magnet with no trackers, so the suite touches no network and behaves the same
in a container as on a workstation.

One command runs the lot, in the build container:

```bash
docker/rust.sh
```

## What the tests found

Four defects, three of them in code written this phase and caught before it
could matter. That is the argument for writing the tests first.

**`alert_cast` never matches a base class.** It compares the concrete alert type
id, so asking it for `torrent_alert` returns null for every torrent-scoped
alert. Every alert crossed the boundary with an empty infohash. `dynamic_cast`
is the right tool for the base class. This would have been very hard to find
later, because the alerts still arrived and still carried their message.

**libtorrent posts `torrent_paused_alert` on a transition, not on the call.** A
torrent paused before it has finished starting never transitions and reports
nothing. The daemon has to account for this when it restores a paused session at
boot, or torrents will silently never report their state.

**A test that passed on arm64 failed on x86-64.** A session raises
`listen_succeeded` while it is still starting, so an assertion about an "idle"
session was really an assertion about timing. Rewritten to assert that
`wait_for_alert` is bounded by its timeout, which is the property the daemon's
alert thread actually depends on. Worth remembering: the second architecture
earns its place in the gate.

**The generated cxx header and the C++ header include each other.** The bridge
emits `using Session = ::redeluge::Session;` inside a header that includes
yours, so the name has to be forward-declared before the generated header is
pulled in. Documented on the [Architecture](Architecture) page because it will come up again
with every new opaque type.

## What the contract extraction found

**`core.get_auth_levels_mappings` is reachable before authentication.** It sits
at auth level 0 on the daemon RPC listener alongside the login call itself. It
leaks nothing sensitive, but an unauthenticated method on that port should be a
deliberate choice, and this one looks incidental. The Rust daemon should put it
at read-only. A test pins the set of unauthenticated methods so a third one
cannot appear unnoticed.

**Five configuration defaults are Python expressions, not literals.** They are
`download_location`, `move_completed_path`, `plugins_location`,
`torrentfiles_location` and `ssl_torrents_certs`, all of which build a path from
the configuration directory. Reproducible in Rust, but they have to be
reimplemented rather than copied. A test pins the set.

**The roadmap said 24 events; there are 22.** The earlier count included the
base class and its metaclass. Corrected.

## Decisions recorded

**No plugin system.** The four functions that matter become part of the daemon.
The ten plugin-management methods are recorded in the contract as removed, with
the reason, rather than silently dropped. One of them, `core.upload_plugin`,
accepted an archive of code that the daemon then executed; that surface goes
away with the mechanism.

**Naming.** New code is `redeluge`. The Python package keeps the `deluge` name
until it is deleted, because renaming a package that is being replaced breaks
the running daemon and buys nothing. Binaries will be `redeluged` and
`redeluge-web`. Configuration file names and formats do not change, so an
existing installation survives the switch.

**libtorrent comes from the distribution.** Debian trixie ships 2.0.11, the
version Deluge is tested against, with the pkg-config metadata the build needs.
No vendoring, no submodule.

## Still open

**The state format, and a sequencing correction.** `torrents.state` is Python
pickle protocol 2. The recommendation stands: convert once with a Python script
rather than teaching Rust to read pickle. The record is flat, twenty-five plain
attributes with two lists, so the converter is short.

The correction: pickle stores a reference to the
`deluge.core.torrentmanager.TorrentState` class, so reading it requires that
module to be importable. The converter therefore has to ship while the Python
tree still exists, which puts it in phase 3, not phase 4. If phase 4 deletes
Python first, anyone who has not migrated is left with a file nothing can read.
The Rust daemon should refuse to start, with a clear message, when it finds a
`torrents.state` and no converted equivalent. Target format: JSON, matching the
configuration files.

**The resume data container should change.** `torrents.fastresume` is one
bencoded dictionary holding every torrent's blob, rewritten whole on each save.
That means Rust would need a bencode reader purely for the envelope, and one bad
write loses the resume data for every torrent at once; the `.bak` file softens
that without removing it. One file per torrent, named by infohash under
`state/resume/`, drops the envelope entirely, confines corruption to a single
torrent, and stops a save of one torrent rewriting the other thousand. It costs
a migration step, and there is already one.

**Resume data now crosses, and it was smaller than expected.** The daemon never
inspects resume data: libtorrent writes it, Deluge stores the bytes, libtorrent
reads them back. So the bridge needs no bencode at all, only
`write_resume_data_buf` on the way out and `read_resume_data` on the way in. Two
functions and a byte slice on the shared struct. Done here rather than deferred
to phase 3, with the full round-trip under test: save, remove the torrent,
re-add it from the bytes alone, same infohash, same save path.

## Ready for phase 1

Phase 1 does not depend on any of this. It needs the DelugeRPC codec, a client
that talks to the existing Python daemon, and the JSON endpoint. The contract is
frozen, the workspace and the test gate exist, and `redeluge-contract` already
reports which methods are still owed:

```rust
let missing = Contract::get().missing_from(implemented_method_names);
```
