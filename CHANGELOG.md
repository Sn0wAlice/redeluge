# Changelog

All notable changes to this project are documented in this file. The format
follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

redeluge numbers its own releases from 1.0.0. The version the daemon reports
to clients stays `2.2.1`, because that is the Deluge a client expects to be
talking to.

## [1.5.0] — 2026-09-16

### Added

- **A running account of what each peer has done**, in a *Peers* window in the
  toolbar, off until you turn it on there. The Peers tab shows the connections
  open right now at the speeds of the moment, which cannot answer the question
  people actually have about an address: what has it ever given back? A peer
  that takes forty gibibytes over a week and sends nothing looks idle in every
  snapshot, because it is idle in every snapshot — libtorrent's own counters
  belong to a connection and die with it.
- Totals are accumulated by difference, never copied: a sample smaller than the
  last one is a reconnection and the whole of it is new. Sampled every fifteen
  seconds, and only for torrents that have peers.
- What sampling cannot see is a connection that begins and ends between two
  samples: nothing reports a peer's totals as it disconnects, so there is no
  other mechanism to use. The bias is harmless — a peer too brief to sample is
  a peer too brief to have taken anything worth the name — and it is documented
  rather than glossed over.
- Kept in `state/peers.json`, so it survives a restart, for thirty days by
  default and at most twenty thousand addresses, the oldest going first. The
  per-connection counters are dropped on the way out, because they describe
  connections that will not exist next time. Nothing about it is sent anywhere
  and no rule acts on it.
- **A Cross-seeds column**: how many of your contents an address carries on
  more than one of your torrents. Not an accusation, and the documentation says
  so: a peer seeding the same release to two trackers uploads real bytes to
  both, most private trackers allow it, and it is what this daemon's own
  tracker rules help you do. It is the quickest way to confirm your own
  cross-seeding is working. Announcing a fake upload figure — the actual way
  ratios are cheated — is invisible from inside a swarm and is the tracker's to
  detect, not a peer's.
- Two torrents of the same content have different infohashes as a matter of
  course, because a tracker stamps its own `source` into the info dictionary.
  So contents are matched on the file list — names and sizes to the byte, in a
  fixed order — which is near enough for grouping and is never used to delete
  anything.
- `redeluge.get_peers` reads the ledger back, biggest taker first.

### Fixed

- **Per-peer byte totals were not carried across the FFI at all.** The bridge
  reported each peer's instantaneous speeds and nothing cumulative, so the one
  number that says what a peer is worth was unavailable to everything above it.

## [1.4.0] — 2026-09-16

### Added

- **A tracker can be given rules of its own.** Right-click a tracker in the
  sidebar and choose *Settings*. A tracker is not something you create, the way
  a label is: it is whatever the torrents you added announce to, and the
  sidebar has been grouping them by it all along. This is somewhere to say what
  should happen to that group, once, rather than on every torrent that arrives
  from it. Stored under the `tracker` key of `core.conf`, so `core.get_config`
  and `core.set_config` configure it like every other feature and no RPC method
  was added.
- **Label them.** A torrent from this tracker gets the label when it arrives,
  when it finishes, or a set number of hours after it finishes, and the label
  is created if it is not there yet. Arrival never overwrites a label you set
  by hand; completion does, which is how a torrent moves from the label it
  downloaded under to the one it is kept under.
- **Move them.** The files go somewhere else a set number of hours after the
  download finished — and the destination disk is measured first, because it is
  not always the disk the files are on now: a folder inside the download folder
  can be a mount point for another drive, which is exactly the case that fills
  one up. A move that would leave under a gibibyte free at the destination is
  not started, is logged once, and is reconsidered on the next pass, so freeing
  space is all it takes. A move within one filesystem is a rename that writes
  nothing and is never refused.
- **Remove them**, a set number of hours after they finished downloading, with
  or without their files. What a tracker asking for a day or a week of seeding
  used to make a chore, and what a disk fills up with when nobody does the
  chore. The files are kept unless that tracker's entry says otherwise, so the
  rule as first turned on does what `remove_at_ratio` does: the torrent goes,
  the download stays.
- Every wait is measured from the completion time libtorrent recorded, which is
  kept in the resume data and survives a restart. Nothing is considered before
  a torrent is finished, and one that is still being moved is left alone until
  it lands. A torrent libtorrent never saw finish — one added over files that
  were already on disk — has no such time: labelling and moving fall back to
  when it was added, and removing does not happen at all, because deleting on a
  wait measured from a time nobody knows is the one way this could take
  something unexpectedly.
- The removal is the one `core.remove_torrent` performs and is announced the
  same way, so every connected client lets go of the torrent instead of holding
  a row for something that no longer exists.

- **The torrent list says what a tracker's rule is about to do**, in a *Tracker
  Rule* column beside *Idle*: *removed in 21h*, *moved in 2h*. Removal is named
  first when both are due, because it is the one that cannot be undone. Counted
  in the browser from two new status fields, `tracker_remove_at` and
  `tracker_move_at`, so it ticks between polls instead of being as old as the
  last one, and both are zero — and the column blank — when nothing is coming.
  A rule that announces itself is a rule you dare leave on.
- **Activity, in the toolbar: what the daemon did without being asked.** Six
  things here act on their own — the share-ratio rule, the idle rule, the
  disk-space rule, the schedule, and a tracker's rules for labelling, moving
  and removing — and until now the only trace any of them left was a line in
  the log, which on a container install means knowing to run `docker logs`.
  The last two hundred actions are kept in memory, newest first, with the
  torrent, the rule and why. It answers *why is that paused* and *what happened
  to that download* beside the torrents rather than in a log file.
- The torrent's name is stored with each entry rather than looked up, because
  after a removal there is nothing left to look it up in.
- `redeluge.get_recent_actions` reads it back. A namespace of this fork's own,
  deliberately not `core.*`, so that no client can mistake it for a Deluge
  method and no future Deluge method can collide with it. It is the only one so
  far, and it is advertised in `daemon.get_method_list` like everything else.

- **The torrents a tracker has dropped are a group of their own**, under
  *States* in the sidebar, beside *Active*: **Unregistered**. A private tracker
  that has pruned a torrent answers every announce with *Unregistered torrent*
  or *Torrent not found* for ever, and until now such a torrent was
  indistinguishable at a glance from one that merely has no peers today.
  Right-click the row and *Remove these torrents...* hands the whole group to
  the ordinary Remove dialog, which asks whether to keep the files as it always
  has. Nothing removes anything on its own.
- The check is deliberately narrow: only the phrases that mean "I have no
  record of this". A tracker that is down, refusing connections or
  rate-limiting is not one that has forgotten the torrent, and offering to
  delete a library because a tracker was rebooting would be unforgivable.
- The Remove dialog now says how many torrents it is about to remove when there
  is more than one, rather than "the torrent (s)".

### Fixed

- **A tracker's own failure reason was thrown away.** For a tracker error the
  daemon reported libtorrent's error code, so a torrent whose tracker had
  dropped it said *Error: tracker failure* — true, useless, and impossible to
  tell apart from any other tracker trouble. The reason the tracker actually
  sent is now what the status carries, falling back to the transport error only
  when the tracker did not answer at all. Tracker warnings carry their text for
  the same reason; they used to arrive empty.

### Changed

- The README now says what this program is and is not: a BitTorrent client
  that ships with no content, no trackers and no search, the responsibility for
  what is done with it, and the warranty that free software does not come with.

## [1.3.1] — 2026-09-12

### Changed

- **Changing a filter is about ten times quicker.** Switching the sidebar from
  one state to another took between half a second and three seconds, measured
  on a library of 400 torrents. Three things were wrong, and all three are
  fixed.
- **A refresh asked for while a poll was in flight was thrown away.** The
  browser refuses to start a second poll, which is right, but it also forgot
  that one had been asked for, so the new filter waited for the next scheduled
  poll. Measured: a click 200 ms into a poll sent nothing at all, and the next
  request went out 2.5 seconds later. The request is now remembered and sent
  the moment the one in flight answers.
- **The session thread answered a call only after its alert wait.** It slept up
  to a hundred milliseconds waiting for libtorrent, then swept the status of
  every torrent, and only then picked up the calls waiting for it, so
  `core.get_external_ip`, which reads one string out of memory, took 115 ms. It
  now waits on the calls themselves, drains all of them that are waiting at
  once, and sweeps every torrent's state on a 250 ms timer rather than on every
  pass. The sweep is the most expensive thing that thread does and it grows
  with the library, so this is less processor as well as less waiting.
- **The status bar's external address, free space and rate limits are no longer
  asked for every two seconds.** Each was a round trip on a connection the
  daemon serves one call at a time, and two of them went through the session
  thread, for answers that are the same for minutes or days. They are held for
  a minute, fifteen seconds and thirty seconds, and dropped the moment the
  configuration is written or the daemon changes.

## [1.3.0] — 2026-09-12

### Added

- **A label can be hidden from the torrent list by default.** Preferences,
  Labels, under *In the torrent list*. The same thing as unticking it under
  *Show labels* in the Label column's header menu, except that it starts that
  way and it is stored with the label, so the next machine you open hides it
  too. What a few hundred torrents in a label you never look at were making
  unusable is the list of the ones you do.
- Nothing is hidden from anything but that list. Picking the label in the
  sidebar shows those torrents, because asking for a label outranks hiding it;
  ticking it in the header menu shows it for the session, and a choice made
  there is not undone by the next reconnect. The sidebar's counts, every other
  client and the daemon itself see no difference.

## [1.2.0] — 2026-09-12

### Added

- **Notifications by webhook**, under Preferences, Notifications, off by
  default. A POST when a download finishes, when a torrent goes into error and,
  if you want it, when one is added. Discord, ntfy, Gotify and a plain JSON
  webhook, as many destinations as you like.
- This is what the Execute plugin was really installed for. Execute ran a shell
  script as the daemon user with the torrent's name as an argument, which is a
  remote code execution primitive wearing a convenience hat; this runs nothing.
- Every kind posts JSON rather than using ntfy's header form, because those
  headers cannot carry anything outside ASCII and half the torrent names that
  matter are not ASCII. Gotify's `/message` is appended for you, a token
  already in the URL is left alone, and an ntfy instance living under a path of
  its own keeps that path.
- **Send test** posts a sample to every destination and writes back what
  happened, under the grid: a wrong URL says so on the page rather than in a
  log nobody is watching. A refusal is not retried, a timeout is.
- It does not announce your library on restart. libtorrent reports a torrent as
  finished again after re-checking one that was already complete, which happens
  to every finished torrent at startup, so a completion older than five minutes
  is not treated as news; nor are the torrents restored at startup.

## [1.1.0] — 2026-09-12

### Added

- **A disk-space rule**, under Preferences, Downloads, and the only feature
  here that is on out of the box. Under 1 GiB free, every torrent still writing
  to that disk is paused; over 2 GiB free, the ones it paused are started
  again. Both numbers are yours to change.
- What it prevents is worth stating, because it is the failure that costs more
  than the download: a disk fills, libtorrent fails a write, that torrent goes
  to Error, and the next one does the same a minute later. Nothing resumes on
  its own once there is room, so it ends with thirty torrents in Error and an
  evening spent working out which stopped for this reason and which for
  another. `core.get_free_space` always knew; nothing ever acted on it.
- It reads each filesystem the session writes to separately, so a download
  landing on a disk with room is not stopped because a different disk is full.
  Seeding torrents are never touched, because a seed writes nothing and taking
  it off the swarm would cost ratio for no gain. Queued and checking torrents
  are, because a queued torrent is one the queue is about to start writing.
- It remembers whether the queue was managing a torrent before it took it, and
  gives that back on release: a torrent you were running by hand does not come
  back under the queue because a disk filled up. A path it cannot measure at
  all, an unplugged disk or an unmounted share, decides nothing either way.
- Two thresholds rather than one, because a single one flaps: pause at 1 GiB, a
  piece is discarded, free space crosses back by a megabyte, everything starts
  and stops again a second later.
- The idle rule now holds on to a download whose time is up while the disk is
  full, instead of starting it for the fifteen seconds it takes this rule to
  stop it again.
- **Where you see it.** A counter in the status bar, beside the idle rule's own
  and hidden the same way when it is holding nothing; clicking it opens the
  settings. A line in the torrent's Status tab saying whether this is what
  stopped it. Pressing Resume ends the hold, though a still-full disk takes it
  back on the next pass.

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

- **A search box**, in the toolbar. The daemon has always been able to search:
  its `keyword` filter covers the name, the state, the tracker and its last
  message, the label and the infohash, with every term having to match. Nothing
  in the interface ever sent it. It narrows whatever the sidebar has selected
  rather than replacing it, so searching inside a label works, and Escape
  clears it.
- **Show or hide labels from the Label column's header menu.** Right-click the
  header and untick a label to take it out of the view, including *No Label*
  for the torrents that have none. It is a view filter applied in the browser,
  not a query: the torrents are already here, it is instant, it survives the
  next poll, and it does not argue with the sidebar's own label filter, which
  answers the different question of which single label to look at.
- **Peer countries can actually be filled in.** The flags were shipped earlier
  in this version; what was missing was the data. Deluge pointed
  `geoip_db_location` at `/usr/share/GeoIP/GeoIP.dat`, a file in the GeoLite
  Legacy format MaxMind retired in January 2019, so that path has held nothing
  for years and Deluge's own flags have been blank just as long. There is now
  an optional downloader, off by default, defaulting to DB-IP's free country
  database: CC BY 4.0, monthly, in the MaxMind DB format the reader already
  takes, and needing no account. It checks weekly, keeps one cached copy, and
  refuses to replace a working database with anything that is not one. A path
  you set yourself still wins.
- **More about each peer.** The country's name in the flag's tooltip, because
  two letters are not something to read. Whether the connection is uTP or TCP
  and whether it is encrypted, both of which libtorrent knew and nothing was
  carrying. And a *Has* column: how many pieces that peer has and we do not,
  which is the one number that says whether a peer is worth having. A seed you
  are already ahead of reads "nothing new"; a peer at three percent may hold
  the piece everything is waiting on.

### Fixed

- **Every session statistic past a certain point was somebody else's.**
  `session_stats_alert` carries one flat array of values and each metric says
  where in it to look; the names were read in list order and counted along it
  instead. The counters and the gauges are numbered in separate ranges, so from
  the point where those diverge every name reported the wrong value: the count
  of connected peers came out as six figures and the DHT node count in the tens
  of thousands. Names are placed at their own index now, and a test asserts
  that for every metric libtorrent publishes rather than for a chosen few.
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
