# Phase 4 report: the switchover

> **Archived.** Part of the Rust migration, which is finished.
> See [Migration Overview](Migration-Overview) for the whole story, and
> the pages under **Using redeluge** for how the software works today.

**Status: done.** The Python implementation is gone. The daemon and the Web UI
are Rust binaries on top of the same libtorrent, an existing installation
converts in one command, and the container image has no interpreter in it.

## What was removed

| | |
|---|---|
| `deluge/` | 28 MB, the whole Python package |
| Build and packaging | `setup.py`, `tox.ini`, `MANIFEST.in`, four `requirements` files |
| Dead tooling | `gen_web_gettext.py`, `generate_pot.py`, `msgfmt.py`, `minify_web_js.py`, `version.py` |
| The Sphinx site | Most of it described a Python project that no longer exists |
| Man pages | They documented binaries that are gone |

Four Python files remain, all under `tools/`: the state converter and the three
generators that produced the frozen contract and the conformance corpora. They
are still linted and formatted in the same gate as the Rust.

What is left is 15 578 lines of Rust across 62 files, plus the contract.

## The image

189 MB, against 416 MB for the Python one. Two binaries and libtorrent; the Web
UI assets are compiled into `redeluge-web` rather than shipped beside it, so
there is no asset path to configure and nothing to forget to copy.

The healthcheck is `redeluge-web --health-check`, which connects to its own
port. Adding curl to the image only to ask it a question it can answer itself
would have been the larger change.

## Migrating an existing installation

`tools/migrate_state.py` converts the torrent list and splits the resume data
into one file per torrent. Nothing is deleted, so running it twice is harmless
and rolling back means not using the new files.

It does not import `deluge`. A pickle names the class it was written from, and
the obvious way to read one is to have that class importable, which would tie
the converter to the thing being removed and strand anyone who upgraded before
converting. Instead the unpickler substitutes a stand-in for any class it meets
and refuses anything that is not a Deluge state class. No code from Deluge ever
runs, which also makes it safe to point at a file from a machine you do not
control.

That removed the sequencing constraint the roadmap had carried since phase 0:
the converter no longer has to ship before the Python tree goes.

Verified against a real configuration the Python daemon produced: two torrents,
one added from a file and one from a magnet, with their per-torrent options and
their resume data. All of it came back in the Rust daemon, including a
`max_connections` of 42 and a stop ratio of 3.5, and the Web UI listed both.

## Four things the switchover exposed

**The Python daemon's certificate is unusable.** It is X.509 version 1 and
rustls will not take one, so every existing installation would have failed to
start. An unusable pair is now moved aside and a new one generated, with the
reason in the log. The certificate proves nothing anyway; no client verifies it.

**Torrent files were never stored.** The daemon added a torrent from a file and
kept no copy, so it could not have restored one after a restart. Now written to
`state/<infohash>.torrent`, named by infohash rather than by the name it was
uploaded under, because two torrents can both arrive as `download.torrent`.
Removing a torrent takes its stored file and resume data with it.

**The restore path looked in the wrong place.** It used the original filename
from the state, where Deluge stores the file under the infohash. Only the
migration of a real configuration would have found this.

**The Web UI's setup script went with the Python.** A Python script had been
seeding the web password and the host entry pointing at the daemon. Deleting it
left `DELUGE_WEB_PASSWORD` doing nothing and nobody able to log in. That logic
now lives in `redeluge-web`, which is where it belongs: it reads the daemon's
own `localclient` credentials out of the auth file and wires the two together
on startup.

## Tests

207 across the workspace, green on both architectures. Five are new, covering
the bootstrap, and the state converter has a self-test in the same gate.

```bash
docker/rust.sh
```

That self-test builds a pickle naming `deluge.core.torrentmanager`, converts
it, and checks the fields survive with the right types. Python refuses to write
a class name it cannot import, so the test puts a stand-in module in
`sys.modules` for the length of the dump. Without that it would have been
testing a shortcut rather than the substitution that matters.

The contract extractor now says so when there is nothing left to extract from:
`contract/*.json` was frozen from a tree that existed and cannot be
regenerated, which beats a gate step failing for a reason nobody remembers.

## What is left

Phase 5, the four functions that were plugins. Labels shape the status API and
were folded into phase 3's contract, so automatic adding, the block list and
the scheduler are what remain.

Plus the leftovers in `TODO.md`. The one that shows first is that the Web UI
does not reconnect to a restarted daemon.
