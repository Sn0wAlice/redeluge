# Phase 3 report: the daemon

> **Archived.** Part of the Rust migration, which is finished.
> See [Migration Overview](Migration-Overview) for the whole story, and
> the pages under **Using redeluge** for how the software works today.

**Status: working, all 70 methods.** `redeluged` answers DelugeRPC, drives
libtorrent through the bridge, keeps its own state across restarts, and the
Rust Web UI runs on it. The stack is Rust from the browser to libtorrent.

## Coverage

| Surface | Contract | Answered |
|---|---|---|
| RPC methods | 70 | 70 |
| Events | 20 | 20 |
| Configuration keys | 77 | 77 |
| Torrent states | 8 | 8 |

The two events the contract lists and this daemon does not send are the plugin
ones, which went with the plugin system.

Authorisation levels are not written in the daemon: they come from
`contract/rpc-api.json`, and a test walks every method to check the level
matches. A method whose level drifted would otherwise be invisible until
someone with a read-only account deleted a torrent.

## What was built

5 203 lines, 614 of them tests.

| File | Lines | What it is |
|---|---|---|
| `src/core.rs` | 1 318 | The 70 methods |
| `src/auth.rs` | 541 | Accounts, scrypt, the auth file |
| `src/manager.rs` | 508 | The session thread, alerts, persistence |
| `src/torrent.rs` | 445 | Per-torrent state and the status dictionary |
| `src/config.rs` | 416 | `core.conf` |
| `src/rpc/server.rs` | 385 | The TLS listener |
| `src/prefs.rs` | 202 | Configuration to libtorrent settings |
| `src/events.rs` | 191 | The events |
| `src/main.rs` | 180 | Startup, shutdown, signals |
| `src/rpc/{tls,dispatch}.rs` | 255 | Certificate and the handler contract |
| `src/state.rs` | 105 | Deriving the state clients see |

## Tests

202 across the workspace, green on x86-64 and arm64.

```bash
docker/rust.sh
```

The one worth naming is `every_contract_method_is_answerable`. It starts a real
daemon and calls all seventy, asserting none replies `NotImplementedError`. Bad
arguments are fine and expected; the point is that the method exists. Checking
this by reading the source would have been a list that drifted.

## Verified live

Against a running daemon, with the Rust Web UI in front of it and a browser on
that:

- The daemon generates its certificate and auth file on first run, listens on
  loopback, and answers `daemon.info` before authentication.
- `daemon.get_method_list` returns exactly the 70 the contract names.
- A magnet added through the Web UI appears in the torrent list with its name,
  progress and state.
- The torrent survives a daemon restart: the state file is read, the torrent is
  re-added from its resume data, and the Web UI shows it again.
- Resume data is written per torrent under `state/resume/`.

## Design decisions

**A thread, not a lock.** The libtorrent session is `Send` but not `Sync`, and
its alert loop parks on a blocking call. Behind a mutex that would either hold
the lock across the wait, stalling every caller, or wake constantly. So the
session lives on one thread that owns it, and callers send it closures. Fifty
operations would otherwise have been fifty command variants for no gain.

**State changes are noticed, not announced.** The session thread sweeps every
torrent each tick and emits `TorrentStateChangedEvent` when one differs from
last time, rather than each operation emitting its own. libtorrent changes state
on its own too: a torrent finishes, the queue starts one, a tracker fails.

**One resume file per torrent.** As the phase 2 report recommended. A bad write
costs one torrent's resume data rather than the whole session's, and no bencode
is needed on the Rust side for the container.

**Torrent state is JSON.** `state/torrents.json`, with the field names the
Python pickle used, so a converted file lines up.

## Three things the tests and the live run found

**`localclient` cannot be hashed.** That account exists so a local tool can log
in by reading the auth file. Hashing its password made it unusable, and the
first run of this daemon locked itself out. It stays in plaintext, alone, and
is never upgraded. What makes that acceptable is the rest: the file is 0600, the
password is twenty random bytes nobody chose, and the account is only reachable
over loopback unless `allow_remote` is on. Every other account is scrypt, and a
plaintext one is rewritten as scrypt the first time it is used.

**The configuration was never written.** Loading fills in every missing key,
which on a first run is all of them, and nothing called save. So a fresh install
had no `core.conf` to edit. Now written on startup and on shutdown.

**Two configuration keys were missing and one was invented.** A test comparing
the defaults against the contract found `ssl_torrents` and `ssl_listen_ports`
absent, `auto_manage_prefer_seeds` listed twice, and a `priority` key that
belongs to a torrent rather than to the daemon.

## Known gaps

**The Web UI does not reconnect to a restarted daemon.** It connects once at
startup and reports "connection lost" afterwards until it is restarted itself.
Visible as soon as the daemon is restarted under it.

**Filtering is by exact match only.** `get_torrents_status` handles the state,
tracker and owner filters every client sends. A client that filtered on a
numeric range would get nothing rather than an error.

**Move-on-completion is stored, not acted on.** The option is kept and reported;
nothing moves a torrent when it finishes yet.

**Ratio-based stopping is stored, not acted on.** Same shape: `stop_at_ratio`
and `stop_ratio` are kept, reported and used to compute the countdown, but no
rule enforces them.

**`core.create_torrent` hashes on a blocking thread and returns the bytes.** It
does not report progress, so `CreateTorrentProgressEvent` is never sent.

## Ready for phase 4

Phase 4 is the switchover: the state converter, deleting the Python tree, and a
single-binary image. The converter is the one piece that has to be written while
Python is still there, and this daemon already refuses to start on an
unconverted `torrents.state` with a message naming it.
