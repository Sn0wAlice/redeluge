# Phase 2 report: the libtorrent bridge

> **Archived.** Part of the Rust migration, which is finished.
> See [Migration Overview](Migration-Overview) for the whole story, and
> the pages under **Using redeluge** for how the software works today.

**Status: complete.** Every libtorrent call the Python daemon makes now has a
Rust equivalent, checked against a real libtorrent 2.0.11 on both
architectures. This is the part the roadmap called the project, and it holds.

## Coverage

The surface was taken from the source rather than guessed at. All of it crosses.

| Surface | Count | Covered |
|---|---|---|
| Torrent handle methods | 40 | 40 |
| Session settings Deluge applies | 29 | all, by name |
| Alert types | 24 | 24 |
| Status fields the daemon reads | 40 | 40 |
| Torrent flags | 9 | 14, the whole set |

Settings are applied by name and type rather than one function per setting, so
the twenty-nine Deluge uses today and whatever it adds later share one code
path. A test walks every name against the linked libtorrent, which is what
turns a renamed setting into a failing build instead of a limit that quietly
stopped working.

## What was built

4 236 lines, 1 355 of them tests.

| File | Lines | What it is |
|---|---|---|
| `src/session.rs` | 553 | The safe Rust API |
| `src/shim_torrent.cc` | 456 | Everything addressed by infohash |
| `src/torrent.rs` | 412 | Status, flags, files, trackers, peers |
| `src/bridge.rs` | 376 | 57 FFI declarations |
| `src/shim.cc` | 294 | Session, settings, adding torrents |
| `src/shim_alert.cc` | 210 | Alert flattening |
| `src/settings.rs` | 142 | Settings by name and type |

## Tests

162 across the workspace, green on x86-64 and arm64, behind the usual gate.

```bash
docker/rust.sh
```

Forty-six of them are new and run against a real session with real torrent
files taken from the Python test data. Everything runs offline: no DHT, no
discovery, no port mapping.

The test worth naming is `every_operation_refuses_an_unknown_infohash`. It walks
the entire surface rather than a sample of it, because one missing guard is a
crash on a torrent the user removed a moment ago.

## Five behaviours of libtorrent the tests uncovered

None of these are bugs in libtorrent. They are the kind of thing that is
obvious in hindsight and expensive to discover from a production incident.

**A rate limit of -1 reads back as 0.** Deluge writes -1 for "no limit" and
libtorrent normalises it to 0, which is how it spells the same thing. So a
setting does not always read back as written, and comparing the two is not a
way to tell whether a change took effect.

**Priority changes are asynchronous.** `prioritize_files` and
`prioritize_pieces` are applied on libtorrent's own thread. Reading the
priorities back immediately returns the old ones. The call is accepted, not
done.

**A name in the add options does not override the metadata.** libtorrent uses
`add_torrent_params::name` only when it has no metadata to take a name from, so
it applies to a bare magnet and not to a torrent file. The daemon cannot rename
a torrent this way; renaming is its own state, kept beside libtorrent.

**`torrent_paused_alert` fires on a transition, not on the call.** Found in
phase 0 and worth repeating: a torrent paused before it has started never
transitions and reports nothing.

**Tracker state moved to the endpoints.** In 2.0 the message, failure count and
updating flag live on each endpoint rather than on the announce entry. The
daemon shows one line per tracker, so the bridge reports the worst endpoint: a
tracker is only healthy when every endpoint reaches it.

## Design decisions worth knowing

**Flags are set and cleared in one call.** `FlagChange` carries both masks
because two calls leave a window where a torrent has neither state. For the
paused flag that window means it starts, announces and stops again.

**Status for every torrent comes back in one crossing.**
`all_torrent_status` exists because the daemon polls all of them on a timer, and
one call per torrent is the difference between a cheap poll and an expensive
one.

**Adding a duplicate is an error.** libtorrent would otherwise return the
existing handle, losing the caller's options while the daemon believed it had
added something.

**Session counters cross on the alert.** They are not readable any other way;
`post_session_stats` asks and `session_stats_alert` answers, carrying the values
alongside the names from `Session::stat_names`.

**The rebuilt `.torrent` is equivalent, not identical.** libtorrent keeps the
parsed metadata rather than the original bytes, so a torrent carrying unusual
extra keys does not round-trip byte for byte. The infohash does, which is what
matters.

## Known gaps

**Peer country is always empty.** It is a GeoIP lookup Deluge does itself, not
something libtorrent reports. It belongs in the daemon, in phase 3.

**Resume data is stored, not inspected.** By design: libtorrent writes it and
reads it back, and nothing in between needs to understand it.

**No SSL torrent has been tested end to end.** The certificate call is wired and
refuses an unknown torrent correctly, but exercising it needs an SSL torrent and
a certificate authority, which belongs with the daemon.

## Ready for phase 3

The daemon is next, and it is the one phase that depends on this one. What it
needs is here: the full handle surface, settings by name, the alert loop, the
status fields, and resume data in and out. What is left for it is the logic that
sits above libtorrent rather than the access to it.
