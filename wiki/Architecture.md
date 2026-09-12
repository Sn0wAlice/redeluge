# Architecture

Two binaries over one C++ library. `redeluged` owns libtorrent and the state;
`redeluge-web` serves the interface and forwards to the daemon over the same
protocol any other client would use. Nothing is shared between them but the
wire.

```
browser  ──HTTP/JSON-RPC──▶  redeluge-web  ──DelugeRPC/TLS──▶  redeluged  ──FFI──▶  libtorrent
thin client ─────────────────────────────DelugeRPC/TLS──────▶
```

## The crates

| Crate | What it is |
|---|---|
| `redeluge-contract` | The frozen RPC surface, embedded at compile time as typed data |
| `redeluge-rencode` | The `rencode` codec Deluge's wire protocol uses |
| `redeluge-rpc` | DelugeRPC: framing, TLS, and a client |
| `redeluge-libtorrent` | Safe bindings to libtorrent, with the C++ shim |
| `redeluge-daemon` | `redeluged`: the methods, the state, the features |
| `redeluge-web` | `redeluge-web`: the JSON endpoint and the embedded front end |

Around 19 000 lines of Rust and C++ across 69 files, plus the contract.

| Path | |
|---|---|
| `contract/*.json` | Generated. The contract itself |
| `tools/` | The only Python left: the extractor, the generators, the state converter |
| `docker/rust.sh` | Runs the whole gate in a container |
| `wiki/` | This documentation |

## The contract

`contract/*.json` is the list of everything the daemon has to reproduce: 99 RPC
methods with their authorisation levels, 22 events, 96 configuration keys, the
24 libtorrent alerts, 73 rencode conformance cases and 10 captured wire frames.

It was extracted from the Python tree by parsing it, never written by hand, and
it is frozen now that the tree is gone. `redeluge-contract` embeds it, so the
daemon asserts what it owes rather than discovering a gap from a client error:

```rust
let missing = Contract::get().missing_from(implemented_method_names);
```

Authorisation levels are read from it rather than written in the daemon. A
level that drifted would otherwise be invisible until someone with a read-only
account deleted a torrent.

## The FFI boundary

One rule: nothing from libtorrent's type system crosses into Rust.

libtorrent alerts are a C++ class hierarchy read with `alert_cast`. Translating
that hierarchy would mean one extern function per alert type and a maintenance
burden on every libtorrent release. Instead the shim flattens each alert into a
plain struct with a discriminant, a message, an optional infohash and a few
payload slots. Rust reassembles that into `Alert` with named accessors.

The discriminants are the order `contract/alerts.json` lists the alerts in.
`src/alert.rs` and `src/shim.cc` both follow that order and a test fails if
they drift, so an alert cannot be handled on one side and forgotten on the
other.

Torrents are addressed by hex infohash rather than by handle, so Rust never
holds a C++ object it could outlive. Errors are thrown as `std::runtime_error`
in the shim and arrive in Rust as `Result::Err`.

### Resume data stays opaque

libtorrent writes it, the daemon stores it, libtorrent reads it back. Nothing
in between looks inside, so it crosses as a byte slice and Rust needs no
bencode of its own. One file per torrent under `state/resume/`, rather than
Deluge's single dictionary rewritten whole on every save, so one bad write
cannot lose the resume data for everything.

### The session lives on its own thread

`lt::session` is `Send` but not `Sync`, and its alert loop blocks. So it sits
on a dedicated thread and everything reaches it as a closure:

```rust
manager.with(|state| state.session.pause_torrent(&id)).await
```

That is the only way to touch libtorrent, which makes it the only place that
has to think about thread safety.

### Three things that bit us

`alert_cast` compares the concrete type and never matches a base class. Asking
it for `torrent_alert` returns null for every torrent-scoped alert, which
crossed the boundary as an empty infohash on every one of them. Use
`dynamic_cast` for a base.

libtorrent posts `torrent_paused_alert` on a state transition, not on the call.
A torrent paused before it started never transitions and reports nothing.

`ip_filter::add_rule` asserts on a range that mixes address families rather
than reporting it, and an assert is compiled out of a release build. The shim
checks first.

## The Web UI

ExtJS 3.4, unchanged from Deluge, compiled into `redeluge-web` rather than read
from a directory: the binary is self-contained and there is no asset path to
configure.

`build.rs` packs `assets/` into one archive and concatenates the two script
bundles, reproducing the order the Python build used. That order matters more
than it looks: ExtJS classes need their base defined first, and a wrong order
produces a bundle of exactly the right size and a blank page. The `.order`
files work backwards, moving each listed name to the front in turn, so the last
name listed ends up first.

The interface is English only. The translation catalogue was a server-rendered
Mako template masquerading as a static asset; dropping it removed the render
step and the class of bug that came with it.

## Naming and compatibility

Binaries and crates are `redeluge`. Configuration file names, their formats and
the directory they live in stay Deluge's, so an existing installation keeps
working. The daemon reports version `2.2.1` to clients, because that is the
Deluge they know how to speak; telling them redeluge's own number makes every
one of them
refuse to connect.
