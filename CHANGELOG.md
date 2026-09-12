# Changelog

All notable changes to this project are documented in this file. The format
follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

redeluge numbers its own releases from 1.0.0. The version the daemon reports
to clients stays `2.2.1`, because that is the Deluge a client expects to be
talking to.

## [1.0.2] — 2026-09-12

### Added

- **A rule that pauses downloads which are getting nowhere**, so the queue can
  move. Off by default. A download under a rate for long enough, with a torrent
  actually waiting for its place, is paused and let go again later. Never the
  last one running, never a torrent taken out of auto-management by hand, and
  downloads only: a torrent that is seeding and transferring nothing is doing
  its job by being reachable.
- The countdown is shown in three places, because the point is to know what is
  about to happen rather than to find out afterwards. An *Idle* column in the
  torrent list, reading "pauses in 4m" and then "resumes in 58m"; a line in the
  torrent's Status tab that says which clock is running and what ends it; and a
  count in the status bar of how many torrents are being held, shown only when
  there are any. All three tick between polls rather than being a sentence the
  server wrote two seconds ago.
- `inactive_down_rate` and `inactive_up_rate` are set from the same threshold
  the rule uses, so libtorrent's own "ignore slow torrents" and this rule agree
  about which torrents are slow. Two mechanisms for the same job, disagreeing
  about the facts, would be impossible to reason about.

### Fixed

- **No pause survived a restart.** Resuming the session resumed every torrent
  it could see rather than the ones that session pause had stopped, and the
  scheduler performs a session resume when the daemon starts. So a torrent
  paused by hand, or stopped at its share ratio, came back running on the next
  restart. It now starts what it stopped and nothing else.
- **Stopping at a share ratio did not stop anything** on a torrent under
  automatic management, which is the default. libtorrent's queue resumes an
  auto-managed torrent it finds paused, within about half a minute, so the
  pause had to clear that flag as well and did not. The same trap is why the
  new idle rule clears it.
- A torrent paused by a rule is now recorded as paused on the torrent itself,
  not only in the session, because a restart re-adds every torrent from what
  was recorded.
- **A caption that wrapped onto a second line was drawn over the control
  below it.** Ext JS pins a checkbox row to the height its config asked for and
  does not clip the label inside it, so a long caption spilled into the next
  row, which had already been positioned. The row is laid out as a flex line
  now, so its height is its tallest child and there is nothing to spill; the
  fixed heights that caused it are gone from thirteen places. Checked at three
  window widths across every preferences page: nothing overlaps.
- **Filtering by tracker matched nothing.** The sidebar and the torrent status
  each worked out the tracker host with their own function, and the two
  disagreed: the list showed `tracker.example.com` while every torrent was
  recorded under `example.com`, so clicking the row returned an empty list. The
  empty case was worse, the list saying `Error` where the status said nothing
  at all. There is one function now, and a test walks every row of the sidebar
  and asserts that filtering on it returns the number of torrents the row
  claims.
- The rule that drops a subdomain was a guess about label lengths, and it got
  `x.abc.com` wrong, leaving it ungrouped, and turned the tracker address
  `192.168.1.1` into `168.1.1`, which is not a host. It is Deluge's own rule
  now: an address is left alone, a two-part public suffix like `co.uk` keeps
  three labels, everything else keeps two.
- A torrent showed no tracker until its first announce succeeded, and sat in
  the sidebar under the torrents that have none. It falls back to the first
  tracker it knows about, which is what Deluge does.
- The sidebar's row for torrents with no tracker was blank, like the label and
  owner rows before it. It reads *No Tracker*.

## [1.0.1] — 2026-09-12

### Added

- **The Label plugin's API, answered without a plugin.** Every program built on
  Deluge asks `core.get_enabled_plugins` whether Label is there and refuses to
  set a download category when it is not: Radarr says "Label plugin not
  activated" under the Category field. This daemon reports it and answers the
  eight `label.*` methods, forwarded through the Web UI's endpoint as well as
  the daemon's own port, because that is what those programs connect to.
  `daemon.get_method_list` grows by exactly those methods, which is what a
  Deluge daemon with the plugin enabled advertises.
- A register of labels, under the `label` key of `core.conf`. A label exists
  whether or not a torrent carries it, which is the whole point: the label an
  external client is about to start using is by definition empty, and a list
  derived from the torrents that happen to exist can never contain it.
- **A Labels page** in Preferences: add, rename, remove, and the per-label
  rules the plugin had, each behind its own switch so that a label which
  changes nothing about the torrents in it stays the default. Rename is three
  existing calls rather than a new method, because the plugin had none.
- **A Label column** in the torrent list, shown by default. The sidebar could
  already count labels and filter on them; nothing could show which torrent
  had which.
- **A Label submenu** on the torrent right-click menu, listing every label with
  the current one marked, and *No Label* to take a torrent out of one. It works
  on a whole selection, and reads the list each time it opens rather than when
  the page loaded, so a label another program has just created is there.
- Two differences from the plugin, both deliberate. `label.add` on a label that
  already exists answers `false` instead of raising, because clients add before
  every use and swallow the error anyway. And `label.set_torrent` with a label
  nobody created **creates it**: the add is the call most likely to have been
  skipped or lost, and refusing means a download silently lands with no
  category.

### Fixed

- **The interface asked for events about twenty times a second.**
  `web.get_events` is a long poll: the front end asks again the instant it is
  answered, which is right for an endpoint that waits and a busy loop for one
  that does not. This server answered empty straight away, so a single open tab
  made roughly six hundred requests a minute at a daemon that had nothing to
  say. The answer is held now until an event arrives or twenty-five seconds
  pass. Measured on an idle tab: 592 requests in thirty seconds before, 28 in
  sixty seconds after.
- **The progress bar was drawn two fifths of its width, with the percentage cut
  off inside it.** The buffered grid view set each cell's style *after* calling
  the renderer rather than before, and the metadata object is one object reused
  across the row, so every renderer saw the previous column's style. The
  progress bar came out as wide as the Size column beside it. The stock
  `Ext.grid.GridView` has always done it the other way round; only this
  vendored subclass did not.
- **A change to anything but the script bundle did not reach a browser.** Asset
  URLs carried the length of the JavaScript bundle, and that same key went on
  the stylesheets, the icons and the other script bundle too, so a fix confined
  to any of those left every URL identical and browsers kept the previous
  build's copy for the hour the cache header allows. Found the hard way: the
  progress-bar fix above was in the image, served, and not running. The key is
  now a digest over every embedded asset.
- **"Active" counted torrents that were not doing anything.** It matched on
  state, so every finished torrent sat in the one category meant for what is
  worth watching. Active is not a state but a question about right now, and
  Deluge asks it as "download or upload rate above zero": a seeding torrent
  nobody is downloading from is idle. The count and the filter use the same
  rule, so they agree.

## [1.0.0] — 2026-09-12

**This is where Deluge became redeluge.**

The fork point is Deluge `2.2.1.dev0-43`, upstream commit
[`e58075416`](https://github.com/deluge-torrent/deluge/commit/e58075416dedd53636e89b1cd240f86f2e7c2ee0).
Everything in this section is the fork. Everything below
[Deluge, before the fork](#deluge-before-the-fork) is upstream history, kept
because it is where the behaviour redeluge reproduces came from.

The Python management layer was replaced by Rust in five phases. libtorrent
stays as it is and is reached through a C++ bridge. The wire protocol, the
configuration files and the Web UI are unchanged, so existing clients and
existing installations keep working.

### Added

- `redeluged`, the daemon: DelugeRPC on port 58846 over TLS, all 70 methods of
  the daemon transport, the 22 events, torrent state, preferences, scrypt
  authentication, and labels.
- `redeluge-web`, the Web UI server: the JSON-RPC endpoint on port 8112 and the
  ExtJS front end, embedded in the binary rather than read from a directory.
- A frozen contract under `contract/`, extracted from the Python tree before it
  was deleted and now the reference the tests check against: 99 RPC methods,
  22 events, 96 configuration keys, 24 libtorrent alerts, 73 rencode
  conformance cases and 10 captured wire frames.
- Six crates: `redeluge-contract`, `redeluge-rencode`, `redeluge-rpc`,
  `redeluge-libtorrent`, `redeluge-daemon`, `redeluge-web`.
- `tools/migrate_state.py`, which converts the pickled torrent list to JSON. It
  is standalone, refuses to unpickle anything but the expected classes, and is
  the only Python that survives.
- A test gate, `docker/rust.sh`: formatting, clippy with warnings denied, the
  test suite, the contract check and the converter self-test, run on x86-64 and
  arm64. 286 tests.
- Container image with no interpreter in it, 189 MB, and systemd units for both
  binaries.
- Per-file `SPDX-License-Identifier` headers, and `AUTHORS` listing upstream
  authorship and the licences of the bundled front end.
- Four of Deluge's plugins as daemon features, off by default and configured
  through `core.conf` rather than through a plugin namespace of their own:
  labels, watched directories, the block list and the weekly schedule. See
  the Features page of the wiki.
- A label is a torrent option, set with `core.set_torrent_options`, reported in
  the status and counted in `core.get_filter_tree` beside state, tracker and
  owner. A label can now be set from the interface as well, in the
  Add dialog and in a torrent's Options tab; until then nothing in the Web UI
  could set one, so the sidebar's Labels list was always empty.
- Controls for the two settings redeluge added: the poll interval under
  Preferences, Interface, and a daemon's certificate fingerprint in the
  Connection Manager's Edit window. Both could only be set by editing
  `web.conf` by hand before.
- A *Fetch Now* button on the Block List page, which brings the next download
  forward without a method for it: it clears the stored timestamp and the
  minute-by-minute check does the rest. It is no longer a second file keyed by torrent id that can drift out
  of step with the first.
- `set_ip_filter` on the libtorrent bridge, which is what the block list
  installs into.
- `.github/workflows/docker.yml`, which builds the image and publishes it to
  the repository's package registry. Started by hand, because publishing
  follows a version bump rather than a push. The tag is the version in
  `[workspace.package]` of `Cargo.toml` and nothing else, and the run stops
  rather than replacing a version already in the registry.
- OCI labels on the image, so the version and the commit it was built from are
  readable without pulling it.
- Documentation as a wiki under `wiki/`, mirrored to the GitHub wiki by a
  workflow on every push that touches it. The repository is the source; pages
  edited in the wiki interface are overwritten.
- `docs/openapi.yaml`: an OpenAPI 3.1 description of the HTTP API, one schema
  per method, generated from the frozen contract by `tools/gen_openapi.py` and
  checked by the gate so it cannot drift.
- `webutils.get_themes` and `webutils.get_languages`, which the contract
  records and the dispatcher did not answer. Aliases of their `web.*` twins, as
  in Deluge.
- Ten Web UI methods that the shipped front end calls and nothing answered, so
  adding a torrent by any route was impossible from the interface and the
  connection manager did nothing: `web.get_torrent_info`,
  `web.get_magnet_info`, `web.download_torrent_from_url`, `web.add_torrents`,
  `web.get_torrent_status`, `web.get_torrent_files`, `web.add_host`,
  `web.edit_host`, `web.remove_host` and `web.stop_daemon`.
- `POST /upload`, the add-by-file endpoint, staging files under the
  configuration directory and refusing anything that is not a torrent.
- Preferences pages for watched folders, the block list and the schedule, the
  last of them a clickable grid of the week.
- The peer list in the torrent status, which was absent entirely, with peer
  countries from a MaxMind DB database when `geoip_db_location` names one.
- Move on completion, and the stop-and-remove-at-ratio rule, both of which were
  stored and reported and never acted on.
- `CreateTorrentProgressEvent`, reported from the hashing loop in C++ through
  the only callback that crosses from C++ into Rust.
- Minified script bundles, and the gzip compression whose feature was enabled
  and whose middleware was never added.
- Rate limiting on the Web UI login: five attempts, then one every thirty
  seconds, per client address.
- Reconnection to a restarted daemon, which previously needed the Web UI
  restarted too.
- Certificate pinning for a remote daemon, through `daemon_fingerprints` in
  `web.conf`.
- Magnet files in a watched directory, and zipped block lists.
- An HTTP integration test that boots the server, and an SSL torrent test that
  generates its own certificate authority.

### Changed

- Labels are a daemon feature rather than a plugin, because they shape the
  status a client asks for.
- The Web UI is English only. The translation catalogue was a server-rendered
  template masquerading as a static asset; dropping it removed the render step
  and the class of bug that came with it.
- Frames are bounded on read: 16 MiB per frame, 64 MiB per decompressed body.
  A compressed body declares its size only after it expands, and 200 KB
  expanding to 200 MB was accepted before.
- The daemon writes its configuration on first start. Loading filled in the
  defaults and nothing saved them, so a fresh install had no `core.conf`.

### Removed

- The Python implementation, in full: daemon, Web UI, GTK and console
  interfaces, plugins, translations, and the setuptools build.
- The plugin interface in the Web UI: the preferences page, the install dialog,
  the loader and the registry. `web.get_plugins` still answers, because it is
  in the contract, and nothing shipped calls it.
- Windows and macOS support. Linux only, from source or from the image.

### Changed

- **The interface is named after the fork.** The toolbar button, the browser
  tab and the About window read `RE:deluge`, and the About window says which
  version of the fork is running rather than only the Deluge version this
  server reports to clients. Every protocol-facing string is untouched: the
  daemon still reports `2.2.1` and still answers Deluge's API, because that is
  what a client written against the Python server expects.
- The Help button opens this project's wiki instead of upstream's user guide,
  and the About window links to the repository.

### Removed from the interface

Each of these was a control whose setting nothing read. The configuration keys
stay, because the daemon answers Deluge's API and a client that asks for them
must get them; only the controls are gone, and
[Configuration](https://github.com/Sn0wAlice/redeluge/wiki/Configuration) lists
every one with its reason.

- The Encryption page and the Cache page, whole. Encryption was never passed to
  libtorrent, and libtorrent 2.0 has no disk cache: it maps files into memory.
- Language, on the Interface page. There is one language.
- Updates and System Information, on the Other page, and the second copy of the
  release check on the Daemon page. Nothing checks for releases and nothing is
  sent anywhere.
- Peer Exchange and the Outgoing Ports group, on the Network page. libtorrent
  2.0 has neither setting.
- Force Use of Proxy. libtorrent 2.0 dropped it; the three switches above it
  are what it meant.
- Port, Enable SSL, Private Key and Certificate, on the Interface page. The
  server has always refused to move its own listener on a browser's say-so.
- Start Daemon, in the Connection Manager. The daemon is a service of its own.

### Fixed

- Choosing a theme in the Web UI stored the first letter of its name.
  `web.get_themes` answered with a flat list of names where the interface
  expects name and label pairs, so ExtJS read each name as a row and took
  character zero as the value: `gray` became `g`, and the page then asked for a
  stylesheet that does not exist. `web.set_theme` now refuses a theme with no
  stylesheet, and the page falls back to the default rather than rendering
  unstyled.
- A missing asset was answered with the page, so a browser asking for a
  stylesheet was handed HTML and reported a parse error rather than a missing
  file. A path that looks like a file now gets a 404; a front-end route still
  gets the page.
- `web.get_config` reported the theme from the startup snapshot rather than the
  live configuration, so the interface showed the previous choice until the
  server was restarted.
- Uploading a torrent from the add dialog failed with "Failed to upload
  torrent". `POST /upload` answered `application/json`, and a form with
  `fileUpload: true` is submitted through a hidden iframe whose document the
  browser builds from the response: only `text/html` makes it insert the body
  unchanged where ExtJS can read it back. The Python server set `text/html` for
  the same reason.
- The interface polled more than it needed to. Nine places call the update
  loop directly, and each one started a poll on top of the pending one, so a
  burst of clicking produced overlapping requests. A poll now replaces the
  pending one and will not start while another is in flight, and the interval
  is `poll_interval` in `web.conf` rather than the number 2000 written into
  five places in the JavaScript.
- Asset URLs carry the build's identity. They are cached for an hour, so an
  upgraded server served new JavaScript that browsers ignored until the cache
  expired, running the old front end against the new API.
- **Seventeen preferences did nothing.** Every torrent was added with a
  hard-coded set of options rather than the configured ones, so the whole "Add
  Torrent Options" group, the per-torrent bandwidth limits and the seeding
  rules could be changed and meant nothing. They are read now, on every route a
  torrent arrives by, and a dictionary the client sends still wins over them.
  `queue_new_to_top`, `copy_torrent_file` and `torrentfiles_location` work too,
  and "prioritise first and last pieces" now actually raises the priority of
  the pieces at each end of each file instead of only being remembered.
- Limits set while adding a torrent were stored and not applied, so a torrent
  added with a speed cap ran uncapped until something set the option a second
  time.
- The three feature preferences pages wrote their settings back even when
  nobody had opened them, which is what the Preferences window does to every
  page on OK. An unopened page holds defaults and an empty grid, so pressing OK
  erased the watched-folder list and reset the weekly schedule. A page that has
  not read its settings now writes nothing.
- A blank number field in those pages serialised as `null`, and `serde` fills
  in a key that is absent rather than one that is present and null, so a single
  null made the daemon discard the whole feature configuration and say so every
  few seconds. Both ends are fixed, and a complaint about a configuration is
  logged once rather than on every pass of the timer.
- The Block List page dropped the two keys the daemon writes, so every Apply
  looked like a list that had never been fetched and the next check downloaded
  it again.
- The watched-folders preferences page threw as it built itself. Its Add button
  handler was called `onAdd`, which is a method `Ext.Container` calls on itself
  every time anything is added to the panel, so the handler ran against a store
  that did not exist yet. A test now refuses either name on the new pages.
- **The preferences window cut its pages off.** The card layout sizes the
  active page to the window, so a page taller than that simply ended below the
  frame with nothing to say so and no way to reach the rest: the Bandwidth page
  lost its per-torrent limits and the Schedule page lost everything below the
  grid. Every page scrolls now, the window is wide enough for the widest of
  them, and it can be resized.
- The watched-folder grid was six hundred pixels wide in a three-hundred-pixel
  page, so three of its six columns were drawn past the frame and could not be
  reached at all. It tracks the page now.
- "Scan every (seconds):" was drawn as three lines with its spinner across
  them, and "Maximum Connection Attempts per Second:" and "Pin sha256:" each
  wrapped onto two. A form lays every field out at the label column's width, so
  a column narrower than the caption does not wrap the caption, it draws the
  field on top of it.
- The add dialog's Options tab had the same cut-off ending as the preferences
  pages.
- A long torrent name was cut at the edge of its cell with no ellipsis, because
  the name is drawn in a block inside the cell and overflowed on its own terms
  rather than the cell's.
- **The Files tab was blank for every torrent.** The daemon never reported
  `files`, `file_progress` or `file_priorities`, so `web.get_torrent_files`
  built an empty tree. They are reported now, and like the peer list they are
  only fetched when a client asks for them.
- The sidebar listed its filter groups alphabetically, which put Labels first
  and States third, because the JSON object they arrive in comes back with its
  keys sorted rather than in the order the daemon wrote them. The Web UI orders
  them itself now.
- Torrents with no label showed as a blank row with a count beside it and
  nothing to say what it was. That group is named now, "No Label" or "No
  Owner".
- The last rows of a long torrent list rendered empty. The buffered grid view
  placed its render window with a constant row height, so any deviation from
  it, a browser zoom, a larger font, a theme with more padding, walked the
  window off the bottom of the viewport, and far enough down the list inverted
  it and blanked every row on screen. It measures the real row pitch now, and
  cannot produce an inverted window.
- Three grid renderers could raise, which stops the grid's render loop and
  empties every row it had not reached, looking exactly like the bug above. The
  torrent name indexed the state without checking it, the progress bar indexed
  a regular expression match without checking it, and the peer flag called a
  string method on a field that may not be sent.
- The peers tab drew its progress bar `NaN` pixels wide. It read `this.width`,
  but a grid renderer is called unbound unless its column sets a scope. Both
  progress renderers now read the width from the metadata the grid passes.
- Peer country flags were broken images: the interface asks for `flag/<code>`
  per row, which is Deluge's own URL, and nothing answered it. The flags are
  shipped and the route validates the code rather than pasting it into a path.
- Tracker icons were a 404 per row on every refresh. Deluge's web server
  fetched each tracker's favicon; redeluge has no such fetcher and will not
  make outbound requests to every host a torrent names, so the grid column and
  the sidebar filter stopped asking for an image that cannot exist.
- The `Allocating` and `Moving` states had no icon in the torrent list or the
  sidebar, leaving an empty indent where one belongs.
- A label or owner containing a quote or a `<` broke the sidebar row it was in:
  the filter template put the value into a class attribute and into the text
  without escaping either.

Defects inherited from upstream and found while porting:

- Two stylesheet references had never resolved, since well before the fork. The
  About window's masthead pointed into a directory the web root does not have,
  so that window has always been blank at the top, and the add dialog's spinner
  was a root-absolute path missing a segment, which would also have broken
  under a base path. A test now walks every `url()` in the stylesheets we own.

- The daemon would not start with pyOpenSSL 26.4, which removed
  `crypto.X509Req`. Rust generates the keypair itself and the dependency is
  gone.
- The daemon certificate Deluge writes is X.509 version 1, which modern TLS
  stacks refuse. An unusable pair is moved aside, not overwritten, and
  regenerated.
- A version string containing `dev` made the Web UI request unbundled sources
  that a release build does not ship.
- `rencode` 1.0.8 hardcoded x86 compiler flags and would not build on arm64.
- Two configuration keys were missing, one was listed twice and one did not
  exist; found by checking the implementation against the contract.

### Migration

- Run `python3 tools/migrate_state.py ~/.config/deluge` once. Nothing is
  deleted, and the daemon refuses to start until it is done rather than
  presenting an empty torrent list.
- `core.conf`, `web.conf`, `hostlist.conf` and the auth file are read as they
  are. A password stored as the old single-round SHA-1 is rewritten as scrypt
  on first use. The `localclient` account stays as it is, because local tools
  read that password back out of the file.

---

## Deluge, before the fork

Everything below is the upstream Deluge changelog at commit `e58075416`,
unchanged.

## Deluge 2.2.1.dev0 (unreleased upstream)

### Breaking changes

- Dropped support for Python 3.8 or older. (Requires Python >= 3.10)

### Core

#### Added

- SSL torrents support for secure peer-to-peer connections. See [libtorrent docs](https://libtorrent.org/manual-ref.html#ssl-torrents) for further implementation details.
- Add option to announce to trackers in all tiers (uTorrent behavior). (#1395)

#### Changed

- Passwords are now stored encrypted with scrypt. A fallback mechanism will still validate existing plaintext passwords in auth files. (#2442)

### GTK UI

#### Fixed

- Fix passwords being ignored in certain dialogs such as Tray Password and Connection Manager.

## 2.2.0 (2025-04-28)

### Breaking changes

- Removed Python 3.6 support (Python >= 3.7)

### Core

- Fix GHSL-2024-189 - insecure HTTP for new version check.
- Fix alert handler segfault.
- Add support for creating v2 torrents.

### GTK UI

- Fix changing torrent ownership.
- Fix upper limit of upload/download in Add Torrent dialog.
- Fix #3339 - Resizing window crashes with Piecesbar or Stats plugin.
- Fix #3350 - Unable to use quick search.
- Fix #3598 - Missing AppIndicator option in Preferences.
- Set Appindicator as default for tray icon on Linux.
- Add feature to switch between dark/light themes.

### Web UI

- Fix GHSL-2024-191 - potential flag endpoint path traversal.
- Fix GHSL-2024-188 - js script dir traversal vulnerability.
- Fix GHSL-2024-190 - insecure tracker icon endpoint.
- Fix unable to stop daemon in connection manager.
- Fix responsiveness to avoid "Connection lost".
- Add support for network interface name as well as IP address.
- Add ability to change UI theme.

### Console UI

- Fix 'rm' and 'move' commands hanging when done.
- Fix #3538 - Unable to add host in connection manager.
- Disable interactive-mode on Windows.

### UI library

- Fix tracker icon display by converting to png format.
- Fix splitting trackers by newline
- Add clickable URLs for torrent comment and tracker status.

### Label

- Fix torrent deletion not removed from config.
- Fix label display name in submenu.

### AutoAdd

- Fix #3515 - Torrent file decoding errors disabled watch folder.

## 2.1.1 (2022-07-10)

### Core

- Fix missing trackers added via magnet
- Fix handling magnets with tracker tiers

## 2.1.0 (2022-06-28)

### Breaking changes

- Python 2 support removed (Python >= 3.6)
- libtorrent minimum requirement increased (>= 1.2).

### Core

- Add support for SVG tracker icons.
- Fix tracker icon error handling.
- Fix cleaning-up tracker icon temp files.
- Fix Plugin manager to handle new metadata 2.1.
- Hide passwords in config logs.
- Fix cleaning-up temp files in add_torrent_url.
- Fix KeyError in sessionproxy after torrent delete.
- Remove libtorrent deprecated functions.
- Fix file_completed_alert handling.
- Add plugin keys to get_torrents_status.
- Add support for pygeoip dependency.
- Fix crash logging to Windows protected folder.
- Add is_interface and is_interface_name to validate network interfaces.
- Fix is_url and is_infohash error with None value.
- Fix load_libintl error.
- Add support for IPv6 in host lists.
- Add systemd user services.
- Fix refresh and expire the torrent status cache.
- Fix crash when logging errors initializing gettext.

### Web UI

- Fix ETA column sorting in correct order (#3413).
- Fix defining foreground and background colors.
- Accept charset in content-type for json messages.
- Fix 'Complete Seen' and 'Completed' sorting.
- Fix encoding HTML entities for torrent attributes to prevent XSS.

### Gtk UI

- Fix download location textbox width.
- Fix obscured port number in Connection Manager.
- Increase connection manager default height.
- Fix bug with setting move completed in Options tab.
- Fix adding daemon accounts.
- Add workaround for crash on Windows with ico or gif icons.
- Hide account password length in log.
- Added a torrent menu option for magnet copy.
- Fix unable to prefetch magnet in thinclient mode.
- Use GtkSpinner when testing open port.
- Update About Dialog year.
- Fix Edit Torrents dialogs close issues.
- Fix ETA being copied to neighboring empty cells.
- Disable GTK CSD by default on Windows.

### Console UI

- Fix curses.init_pair raise ValueError on Py3.10.
- Swap j and k key's behavior to fit vim mode.
- Fix torrent details status error.
- Fix incorrect test for when a host is online.
- Add the torrent label to info command.

### AutoAdd

- Fix handling torrent decode errors.
- Fix error dialog not being shown on error.

### Blocklist

- Add frequency unit to interval label.

### Notifications

- Fix UnicodeEncodeError upon non-ascii torrent name.

## 2.0.5 (2021-12-15)

### WebUI

- Fix js minifying error resulting in WebUI blank screen.
- Silence erronous missing translations warning.

## 2.0.4 (2021-12-12)

### Packaging

- Fix python optional setup.py requirements

### Gtk UI

- Add detection of torrent URL on GTK UI focus
- Fix piecesbar crashing when enabled
- Remove num_blocks_cache_hits in stats
- Fix unhandled error with empty clipboard
- Add torrentdetails tabs position menu (#3441)
- Hide pygame community banner in console
- Fix cmp function for None types (#3309)
- Fix loading config with double-quotes in string
- Fix Status tab download speed and uploaded

### Web UI

- Handle torrent add failures
- Add menu option to copy magnet URI
- Fix md5sums in torrent files breaking file listing (#3388)
- Add country flag alt/title for accessibility

### Console UI

- Fix allowing use of windows-curses on Windows
- Fix hostlist status lookup errors
- Fix AttributeError setting config values
- Fix setting 'Skip' priority

### Core

- Add workaround libtorrent 2.0 file_progress error
- Fix allow enabling any plugin Python version
- Export torrent get_magnet_uri method
- Fix loading magnet with resume_data and no metadata (#3478)
- Fix httpdownloader reencoding torrent file downloads (#3440)
- Fix lt listen_interfaces not comma-separated (#3337)
- Fix unable to remove magnet with delete_copies enabled (#3325)
- Fix Python 3.8 compatibility
- Fix loading config with double-quotes in string
- Fix pickle loading non-ascii state error (#3298)
- Fix creation of pidfile via command option
- Fix for peer.client UnicodeDecodeError
- Fix show_file unhandled dbus error

### Documentation

- Add How-to guides about services.

### Stats plugin

- Fix constant session status key warnings
- Fix cairo error

### Notifications plugin

- Fix email KeyError with status name
- Fix unhandled TypeErrors on Python 3

### Autoadd plugin

- Fix magnet missing applied labels

### Execute plugin

- Fix failing to run on Windows (#3439)

## 2.0.3 (2019-06-12)

### Gtk UI

- Fix errors running on Wayland (#3265).
- Fix Peers Tab tooltip and context menu errors (#3266).

### Web UI

- Fix TypeError in Peers Tab setting country flag.
- Fix reverse proxy header TypeError (#3260).
- Fix request.base 'idna' codec error (#3261).
- Fix unable to change password (#3262).

### Extractor plugin

- Fix potential error starting plugin.

### Documentation

- Fix macOS install typo.
- Fix Windows install instructions.

## 2.0.2 (2019-06-08)

### Packaging

- Add systemd deluged and deluge-web service files to package tarball (#2034)

### Core

- Fix Python 2 compatibility issue with SimpleNamespace.

## 2.0.1 (2019-06-07)

### Packaging

- Fix `setup.py` build error without git installed.

## 2.0.0 (2019-06-06)

### Codebase

- Ported to Python 3

### Core

- Improved Logging
- Removed the AutoAdd feature on the core. It's now handled with the AutoAdd
  plugin, which is also shipped with Deluge, and it does a better job and
  now, it even supports multiple users perfectly.
- Authentication/Permission exceptions are now sent to clients and recreated
  there to allow acting upon them.
- Updated SSL/TLS Protocol parameters for better security.
- Make the distinction between adding to the session new unmanaged torrents
  and torrents loaded from state. This will break backwards compatibility.
- Pass a copy of an event instead of passing the event arguments to the
  event handlers. This will break backwards compatibility.
- Allow changing ownership of torrents.
- File modifications on the auth file are now detected and when they happen,
  the file is reloaded. Upon finding an old auth file with an old format, an
  upgrade to the new format is made, file saved, and reloaded.
- Authentication no longer requires a username/password. If one or both of
  these is missing, an authentication error will be sent to the client
  which should then ask the username/password to the user.
- Implemented sequential downloads.
- Provide information about a torrent's pieces states
- Add Option To Specify Outgoing Connection Interface.
- Fix potential for host_id collision when creating hostlist entries.

### Gtk UI

- Ported to GTK3 (3rd-party plugins will need updated).
- Allow changing ownership of torrents.
- Host entries in the Connection Manager UI are now editable.
- Implemented sequential downloads UI handling.
- Add optional pieces bar instead of a regular progress bar in torrent status tab.
- Make torrent opening compatible with all Unicode paths.
- Fix magnet association button on Windows.
- Add keyboard shortcuts for changing queue position:
  - Up: `Ctrl+Alt+Up`
  - Down: `Ctrl+Alt+Down`
  - Top: `Ctrl+Alt+Shift+Up`
  - Bottom: `Ctrl+Alt+Shift+Down`

### Web UI

- Server (deluge-web) now daemonizes by default, use '-d' or '--do-not-daemonize' to disable.
- Fixed the '--base' option to work for regular use, not just with reverse proxies.

### Blocklist Plugin

- Implemented whitelist support to both core and GTK UI.
- Implemented IP filter cleaning before each update. Restarting the deluge
  daemon is no longer needed.
- If "check_after_days" is 0(zero), the timer is not started anymore. It
  would keep updating one call after the other. If the value changed, the
  timer is now stopped and restarted using the new value.
