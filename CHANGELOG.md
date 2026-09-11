# Changelog

All notable changes to this project are documented in this file. The format
follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

redeluge numbers its own releases from 0.1.0. The version the daemon reports
to clients stays `2.2.1`, because that is the Deluge a client expects to be
talking to.

## [0.1.0] — 2026-09-11

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
  owner. It is no longer a second file keyed by torrent id that can drift out
  of step with the first.
- `set_ip_filter` on the libtorrent bridge, which is what the block list
  installs into.
- Documentation as a wiki under `wiki/`, mirrored to the GitHub wiki by a
  workflow on every push that touches it. The repository is the source; pages
  edited in the wiki interface are overwritten.
- `docs/openapi.yaml`: an OpenAPI 3.1 description of the HTTP API, one schema
  per method, generated from the frozen contract by `tools/gen_openapi.py` and
  checked by the gate so it cannot drift.
- `webutils.get_themes` and `webutils.get_languages`, which the contract
  records and the dispatcher did not answer. Aliases of their `web.*` twins, as
  in Deluge.

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

### Fixed

Defects inherited from upstream and found while porting:

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
