# Phase 1 report: the Web UI in Rust

> **Archived.** Part of the Rust migration, which is finished.
> See [Migration Overview](Migration-Overview) for the whole story, and
> the pages under **Using redeluge** for how the software works today.

**Status: working.** `redeluge-web` serves the same interface the Python
`deluge-web` serves, talks to the Python daemon over DelugeRPC, and the browser
cannot tell the difference. The Python web server is no longer needed.

The daemon on the other end is still Python, deliberately. That is what made
every piece of protocol code checkable against a reference implementation
instead of against itself.

## What was built

4 603 lines of Rust and 178 of build script, 1 240 of it tests, plus 263 lines
of Python that generate the conformance corpora.

| Crate | Source | Tests | What it is |
|---|---|---|---|
| `redeluge-rencode` | 813 | 445 | The rencode wire format |
| `redeluge-rpc` | 916 | 286 | Framing, messages, TLS, the daemon client |
| `redeluge-web` | 2 193 | 509 | The JSON endpoint, sessions, assets |

The Web UI assets now live in `crates/redeluge-web/assets`, 664 files and
5.5 MB, and are embedded in the binary at build time. Deleting the Python tree
later cannot take the interface with it, and the server is one file with no
asset path to configure.

## Tests

116 across the workspace, green on both architectures, behind the same gate as
phase 0: `cargo fmt --check`, `cargo clippy -D warnings`, the full suite, and
the contract check.

```bash
docker/rust.sh
```

The two that matter most are conformance suites rather than unit tests.

**rencode is checked against the Python module, not against prose.** rencode has
no specification; `rencode_orig.py` is the specification. So
`tools/gen_rencode_corpus.py` runs the real implementation over 73 values that
each pin a boundary in the format, and the Rust tests assert both directions.
Our encoder produces Python's exact bytes for all 73, and our decoder produces
Python's exact values.

**The framing is checked against frames the daemon actually emits.**
`tools/gen_rpc_frames.py` uses deluge's own transfer code path, so the ten
frames in the corpus are the bytes that go over the wire.

**The script bundle is byte-identical to the Python build.** `build.rs`
reproduces the concatenation order of `minify_web_js.py`, `.order` files
included, and a comparison against the Python-built bundle matches exactly.

## Live verification

Against a running Python daemon, with a real browser:

- TLS handshake, `daemon.info` before authentication, login at auth level 10.
- The daemon exposes 76 methods; the contract expects 70. The difference is
  exactly the six plugin-management methods removed on purpose.
- Log in through the web page, get the torrent list, the filter sidebar, and a
  status bar with live rates, DHT node count and free space.
- `core.*` calls proxied to the daemon and answered with real data.
- An unauthenticated call refused with code 1, a wrong password refused.

## What the work found

**The shipped `gettext.js` is a server-rendered template, not a static asset.**
Its catalogue maps every string to a Mako expression, which the Python server
substituted on each request. Serving it as a static file turned every label in
the interface into the raw expression. Dropping translation, which is now the
decision, removes the catalogue, the extraction tool, the render step and this
whole class of bug. `_()` is the identity function and the front end needed no
changes. A test asserts that no shipped asset still carries an unrendered
marker.

**`render/*.html` fragments carry the same markers.** They are resolved at
request time by the template engine, which treats `_("text")` as `text`.

**The `.order` file works backwards.** Each listed name is moved to the front in
turn, so the last one listed ends up first. Getting that wrong produced a bundle
of exactly the right size and the wrong order, which is the worst kind of wrong
and was caught only by a byte comparison.

**The front end calls `web.get_plugins` on load and throws without it.** It is
answered with an empty list, which is truthful: there are no plugins. The call
disappears when the plugin UI does, in phase 5.

## Differences from the Python server, on purpose

| Area | Python | Here |
|---|---|---|
| Web password | one round of SHA-1, `==` comparison | scrypt, constant time, legacy hashes upgraded on first login |
| Sessions | written into `web.conf` | in memory only |
| Session cookie | no flags | `HttpOnly`, `SameSite=Strict` |
| Frame size | 32-bit length accepted as given | capped before anything is reserved |
| Decompression | unbounded | capped, so a compression bomb fails |
| Daemon traceback | forwarded to the browser | dropped |
| Translation | Mako plus a catalogue | English only |

The password upgrade is the one users will notice, and only in the logs: an
existing installation keeps working, and the stored hash is rewritten as scrypt
the first time someone signs in. Verified end to end against a real `web.conf`.

## Not done yet

**Torrent upload.** `POST /upload`, which the add-by-file dialog uses. Adding by
magnet and by URL work, because those are daemon calls.

**Minified bundles.** Only the concatenated ones are produced, so the browser
downloads roughly twice what it needs to. Correctness first; the script set is
chosen by what is actually present, so adding minification changes nothing else.

**Event delivery is polled, as it was.** The daemon pushes events over RPC and
they are queued for `web.get_events`, which is what the front end asks for. A
push transport would mean changing the front end.

## Ready for phase 2

Phase 1 depended on nothing from phase 0 and phase 2 depends on nothing from
phase 1, which is why they were separable. The libtorrent bridge is next, and
the pieces it needs are already there: the contract names the 40 handle methods
and the 24 alerts, and the spike proved the pattern.
