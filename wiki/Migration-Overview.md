# Migration Overview

> **Archived.** The migration is finished. These pages are kept because they
> record why the software is shaped the way it is, and what went wrong on the
> way. For how redeluge works today, start at [Home](Home).

redeluge began as a fork of Deluge at `2.2.1.dev0-43`, upstream commit
[`e58075416`](https://github.com/deluge-torrent/deluge/commit/e58075416dedd53636e89b1cd240f86f2e7c2ee0).
The Python management layer was replaced by Rust in six phases. libtorrent was
never rewritten: it stays in C++ behind an FFI bridge.

The wire protocol, the configuration files and the Web UI came through
unchanged, which was the constraint the whole plan was built around.

## The phases

| | | |
|---|---|---|
| [Phase 0](Migration-Phase-0) | Scoping and the bridge spike | The contract frozen, `cxx` to libtorrent proven on two architectures |
| [Phase 1](Migration-Phase-1) | The Web UI server | rencode, DelugeRPC, and a Rust `deluge-web` driving the Python daemon |
| [Phase 2](Migration-Phase-2) | The libtorrent bridge | 40 handle methods, 29 settings, 24 alerts, 40 status fields |
| [Phase 3](Migration-Phase-3) | The daemon | All 70 methods, the events, the state, the configuration |
| [Phase 4](Migration-Phase-4) | The switchover | Python deleted, a converter for existing installations, a 189 MB image |
| [Phase 5](Migration-Phase-5) | The four features | Labels, watched directories, block list, schedule |

Two lanes ran in parallel until phase 3: the client lane delivered something
usable before the engine lane compiled.

## What made it work

**A frozen contract.** Before anything was written, the Python tree was parsed
and its surface written out as data: 99 RPC methods with their authorisation
levels, 22 events, 96 configuration keys, 24 libtorrent alerts, 73 rencode
conformance cases and 10 captured wire frames. The Rust implementation is
checked against that file rather than against itself. Two missing configuration
keys, one duplicated key and one invented key were found this way.

**Differential testing.** Each stage ran against the previous one: the Rust
client against the Python daemon, the Rust web server against the Python
daemon, then against the Rust daemon, and finally a real Python configuration
migrated and running in the interpreter-free image.

**Deleting as we went.** Files were copied into the Rust crates when they were
first used rather than at the end, so nothing was left stranded when the Python
tree was removed.

## What the phases actually cost

The estimates in the original plan were in weeks per phase. What they bought
was not speed but a list of the traps, and the traps were where the time went:

- `alert_cast` never matches a base class, so every alert crossed the FFI
  boundary with an empty infohash.
- A test passed on arm64 and failed on x86-64 because a session raises
  `listen_succeeded` while still starting.
- The `.order` files that drive the front-end bundle work backwards, and
  getting it wrong produces a bundle of exactly the right size and a blank
  page.
- `gettext.js` was a server-rendered template masquerading as a static asset.
- The `localclient` account cannot be hashed without locking the daemon out of
  itself.
- Deluge's daemon certificate is X.509 version 1, which rustls refuses, so
  every existing installation would have failed to start.

Each of those is written up in the phase where it was found.

## What it did not deliver

Three things were stated at the outset and held true:

- **No performance gain.** The real work is done by libtorrent, in C++, before
  and after. What changed is maintainability and deployment size.
- **No removal of a native dependency.** A Python glue maintained by others was
  replaced by an FFI glue maintained here. Defensible; not a simplification.
- **No graphical or console interface.** They went with Python.

## The roadmap as it was drawn

The original plan, annotated with what actually happened in each phase, is
[published as a page](https://claude.ai/code/artifact/139ff1f9-0ef4-4e38-9c2a-9a2fcea08027)
in French.
