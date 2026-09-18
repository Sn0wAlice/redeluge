# TODO

Everything known to be outstanding, from the phases done so far. The roadmap
says what the phases are; this says what is actually left.

Items are grouped by when they have to be done, not by size. Anything marked
**blocking** stops a later phase from being correct. Ticked items are done and
stay here until the section is emptied, so the record of what was outstanding
survives the fixing of it.

---

## Making a torrent

Done, end to end: the `Create` button beside `Remove` opens a window that
browses the daemon's disk, hashes what you choose, and hands the file back to
the browser, writes it on the daemon, seeds it, or all three.

- [x] **A way for the browser to receive the file.** `GET /created/<job>` on
      the Web UI server, which takes the bytes straight off the wire rather
      than through `rencode_to_json`, whose `from_utf8_lossy` would replace
      most of a file of SHA-1 hashes.
- [x] **The dialog.** `CreateTorrentWindow.js`, two tabs. The toolbar button
      Deluge shipped hidden is where it always was, unhidden and wired.
- [x] **A path chooser that can pick a file.** `redeluge.list_directory`, which
      answers with files as well as directories and says which is which.
      `core.get_completion_paths` stays as it was: its shape is Deluge's, and a
      client written against it would not survive a new one.
- [ ] **One call still monopolises a connection.** `core.create_torrent` blocks
      the connection it arrived on for the length of the hashing, because the
      listener awaits each message inline — which also means the event branch
      of its `select!` is not polled, so progress events to that client queue up
      behind the call they are about. The job form sidesteps it rather than
      fixing it; anything else long-running will meet it again.
- [ ] **`FileBrowser.js` is still a stub** inherited from upstream: forty-three
      lines, four toolbar buttons, no behaviour. Nothing references it, and the
      create window has its own browser. It should go, or become the shared one.

## Phase 1 leftovers

These make the Rust Web UI a complete replacement rather than a working one.

- [x] **Torrent upload, and nine other methods that were missing with it.**
      This item said the upload endpoint was the only gap. It was not: ten
      methods the shipped front end calls were not answered at all, so adding a
      torrent by *any* route was impossible from the interface, as was the
      whole connection manager. `POST /upload` now stages files under the
      configuration directory, and `web.get_torrent_info`, `web.get_magnet_info`,
      `web.download_torrent_from_url`, `web.add_torrents`,
      `web.get_torrent_status`, `web.get_torrent_files`, `web.add_host`,
      `web.edit_host`, `web.remove_host` and `web.stop_daemon` all answer.
      Reading a torrent needed bencode, which the daemon had deliberately
      avoided; it is in the web crate, where the add dialog needs a file's name
      and tree before anything is added.
- [x] **Minified script bundles, and the compression that was never on.**
      `build.rs` now emits a minified bundle beside the debug one, which is
      what `ScriptSet::Normal` was already looking for: 294 KB to 159 KB, and
      91 KB to 47 KB. The minifier is deliberately conservative, stripping
      comments and indentation but keeping line breaks so automatic semicolon
      insertion behaves; it is in the library so its tests run, and the ones
      that matter are the regular expression and string cases that eat a file
      when they are wrong. Separately, the `compress-gzip` feature was enabled
      and the middleware was never added, so nothing had ever been compressed.
- [x] **An integration test that boots the server.** `tests/http.rs` boots the
      same application the binary serves, through the same
      `routes::configure`, and drives it: the page, the assets, the fragments,
      the session cookie, the throttle, the upload path and the host list. One
      test asserts every method the front end calls is answered, which is the
      test that would have caught the ten missing ones.

## Phase 3 leftovers

All are behaviour the Python daemon has and this one does not yet.

- [x] **The Web UI does not reconnect to a restarted daemon.** A supervisor
      task notices the closed connection and reconnects, backing off from five
      seconds to a minute so a daemon that is down for an hour does not mean an
      hour of attempts every five seconds. There is no heartbeat in DelugeRPC,
      so a dead connection is only visible as a closed channel; `Client` now
      exposes that.
- [x] **Move on completion.** A torrent that finishes is moved, if it has a
      destination. Only on the transition: libtorrent posts the same alert
      after a recheck of something already complete, and moving every time
      would move the files out from under themselves on each restart.
- [x] **Stop and remove at ratio.** Enforced on a five-second sweep rather than
      on an alert, because a ratio creeps past its limit while nothing happens
      and there is no event to hang it on. Removing is checked before pausing,
      which is Deluge's order, and the data is never deleted.
- [x] **Progress while creating a torrent.** The hashing loop in C++ now calls
      back into Rust, which is the only thing that crosses the boundary in that
      direction. Throttled to a hundred events for the whole run: a torrent can
      have hundreds of thousands of pieces.
- [x] **Filtering is exact-match only.** `keyword` now searches the name,
      state, tracker, tracker message, label and infohash, with every term
      having to match, and `name` is a substring rather than an exact match.
      The one field upstream searched that this does not is the file list:
      fetching every torrent's files to answer a keystroke is a great deal of
      work for a search box.

## Phase 2 leftovers

Small, and all of them belong to the daemon rather than to the bridge.

- [x] **Peer country, and the peer list it belongs to.** This said the country
      was empty. It was worse: the status carried no `peers` key at all, so the
      peers tab of the interface showed nothing. The list is there now, fetched
      only when a client asks for that key, and the country is filled in from a
      MaxMind DB database if `geoip_db_location` names one. The legacy
      `GeoIP.dat` Deluge defaulted to is retired and is reported as such rather
      than failing to parse. No database is shipped: its licence forbids it.
- [x] **An SSL torrent, end to end.** `tests/ssl.rs` generates a certificate
      authority, writes a torrent carrying it, adds it, and waits for
      `torrent_need_cert_alert`, which is the observable proof the torrent was
      recognised as an SSL one rather than added with its extra field ignored.
      Then it sets a certificate the authority signed and asserts it is
      accepted. Everything is generated in the test, because a certificate in
      a fixture expires.

## Phase 5 leftovers

The four features landed. These are the edges of them.

- [x] **A Web UI for the three settings.** Three preferences pages: watched
      folders as an editable grid, the block list with its URL and whitelist,
      and the schedule as a clickable 24 by 7 grid that cycles each hour
      through full speed, slow and stopped. Each page reads and writes its one
      dictionary through `core.get_config` and `core.set_config` rather than
      binding flat keys, because that is the shape the settings have. Labels
      still need no page: they are a torrent option and the sidebar draws them
      on its own.
- [x] **Magnet files in a watched directory.** Read, one link per line, blank
      lines and comments skipped. The file is one unit for disposal, so a bad
      link among several does not leave the good ones to be re-added on every
      scan.
- [x] **Zip block lists.** Read, taking the largest member because these
      archives often carry a readme beside the list, and refusing one that
      declares an absurd uncompressed size. bzip2 is still refused by name: it
      would mean a C dependency for a format no list actually uses.
- [x] **The block list is not re-read when its URL changes.** Checked every
      minute now rather than hourly, and a changed URL forces a fetch. A
      changed whitelist reinstalls from the cached list without downloading
      anything, because the list is still the right list.
- [x] **A schedule change applies on the hour.** It now wakes every minute as
      well as on the hour. Nothing happens unless the state actually differs,
      so the extra wake-ups cost nothing and someone who just edited the grid
      sees it mean something.

## The front-end display audit

Everything a read of the interface's stylesheets and renderers turned up, after
the report that rows at the bottom of a long torrent list went blank.

- [x] **Blank rows at the bottom of a long list.** The buffered grid view
      computed its render window from a constant row height, and any deviation
      from it, a browser zoom, a larger font, a theme with more padding, shifted
      the window until it no longer covered the viewport, and at a large enough
      offset inverted it and blanked every visible row. It now measures the real
      pitch across the rendered rows, keeps four rows of slack, and cannot
      produce an inverted window.
- [x] **Renderers that could throw and empty the rows after them.** A grid
      renderer that raises stops the render loop, which looks exactly like the
      bug above. The torrent name indexed `state` without checking it, the
      torrent progress indexed a regular expression match without checking it,
      and the peer flag called a string method on a field the daemon need not
      send. All three are guarded.
- [x] **The peers progress bar was drawn `NaN` pixels wide.** It read
      `this.width`, but a grid renderer is called unbound unless the column
      sets a scope, so the width was `undefined`. Both progress renderers now
      read the width the grid actually used, from the metadata it passes.
- [x] **Peer country flags were broken images.** The peers tab asks for
      `flag/<code>` per row, which is Deluge's own URL, and nothing served it.
      The 247 flags are shipped and the route answers them, with the code
      validated rather than pasted into a path.
- [x] **Tracker icons were a 404 per row per refresh.** Deluge's web server
      fetched each tracker's favicon and cached it. There is no such fetcher
      here and there should not be one, so the grid column and the sidebar
      filter no longer ask for an image that will never exist.
- [x] **Two stylesheet references that had never resolved.** The About window's
      masthead pointed into a directory the web root never had, and the add
      dialog's spinner was a root-absolute path missing a segment, which would
      also have broken under a base path. A test now walks every `url()` in our
      own stylesheets.
- [x] **Two torrent states had no icon.** `Allocating` and `Moving` reached
      both the name cell and the sidebar, and neither had a rule, so they drew
      twenty pixels of empty indent.
- [x] **A label could break the sidebar row it was in.** The filter template
      put the value into a class attribute and into the text unescaped. Both
      are escaped now, the class through a formatter that keeps only what is
      valid in one.

## The settings audit

Every preference in the interface, checked against what the daemon and
libtorrent actually do with it. Two lists came out of it and both are done.

- [x] **Nine groups of controls that could not mean anything, removed.** The
      Encryption page and the Cache page whole, the language selector, the
      release check in both places it appeared, the statistics upload, Peer
      Exchange, the outgoing port range, Force Use of Proxy, the four
      server-binding fields the server has always refused to change, and Start
      Daemon. The configuration keys stay: the daemon answers Deluge's API and
      a client that reads them must keep working. A test walks the shipped
      bundle and fails if any of them comes back.
- [x] **Seventeen settings that were stored and never read, wired up.** A
      torrent was added with a constant set of options rather than the
      configured ones, so the whole "Add Torrent Options" group, the four
      per-torrent limits and the seeding rules did nothing. They are read on
      every route a torrent arrives by, including watched directories, and what
      the client sends still wins. `queue_new_to_top`, `copy_torrent_file` and
      `torrentfiles_location` work now, and prioritising first and last pieces
      actually changes piece priorities rather than only being remembered.
- [x] **Labels had no interface.** The daemon has had them since the plugins
      became features and the sidebar could always filter on one, but nothing
      could set one, so that filter was permanently empty. There is a field in
      the Add dialog and in a torrent's Options tab.
- [x] **The two settings redeluge added had no control.** The poll interval is
      on the Interface page and a daemon's certificate fingerprint is in the
      Connection Manager's Edit window. Both were file-only.
- [x] **A block list that would not refresh.** A *Fetch Now* button, which
      clears the stored timestamp rather than calling a method that does not
      exist. Inventing one would put the daemon's API out of step with Deluge's,
      which is the thing this whole project is arranged not to do.
- [x] **Preferences erased what it had not read.** The three feature pages
      wrote their settings back on OK whether or not anyone had opened them,
      and an unopened page holds defaults and an empty grid. That erased the
      watched-folder list and reset the weekly schedule. It also wrote `null`
      for every blank number field, which made the daemon discard the whole
      feature configuration and complain about it every few seconds.

## The second display pass

Found by driving the running interface rather than by reading it: a throwaway
instance, sixty torrents, and a probe that walks the DOM of every page, tab and
window and reports anything drawn past an ancestor that clips without
scrolling. It reports nothing now.

- [x] **The preferences window cut its pages off.** The card layout sizes the
      active page to the window, so anything taller ended below the frame with
      no scrollbar and no way to reach it. The Bandwidth page lost its
      per-torrent limits that way. Every page scrolls, the window fits the
      widest of them, and it resizes.
- [x] **The watched-folder grid was drawn past the frame**, six hundred pixels
      of it in a three-hundred-pixel page, so half the columns were
      unreachable.
- [x] **Three captions wrapped under their own fields.** A form puts every
      field at the label column's width, so a column narrower than the caption
      draws the field on top of it rather than widening.
- [x] **The add dialog's Options tab** ended the same way the preferences pages
      did.
- [x] **A long torrent name had no ellipsis.** The name is a block inside the
      grid cell, and it overflowed on its own terms rather than the cell's.
- [x] **The Files tab was blank for every torrent.** Not a layout problem: the
      daemon never reported the three file keys at all, so the tree the Web UI
      builds was always empty.
- [x] **The sidebar listed its filter groups alphabetically**, Labels first and
      States third, because the JSON object comes back with sorted keys. The
      order is the Web UI's own now.
- [x] **An unnamed filter row.** Torrents with no label formed a group with a
      blank name and a count beside it.

## The third display pass

Three things reported from a real install, all measured rather than guessed.

- [x] **Six hundred requests a minute for events.** `web.get_events` is a long
      poll and this server answered it immediately, so the front end's
      ask-again-on-answer loop spun as fast as the round trip allowed. The
      answer is held now, up to twenty-five seconds, and an event still ends
      the wait at once. 592 requests in thirty seconds became 28 in sixty.
- [x] **The progress bar and its text.** The buffered grid view assigned the
      cell style after calling the renderer, and the metadata object is reused
      down the row, so every renderer read the previous column's width. The bar
      was as wide as the Size column and the percentage was clipped inside it.
- [x] **A fix that was shipped, served, and invisible.** Asset URLs were keyed
      on the JavaScript bundle's length alone, and the same key went on every
      asset, so a change to a stylesheet or to the other script bundle changed
      no URL at all. Found the hard way: the progress-bar fix was in the image
      and the browser kept running the old one. Keyed on every asset now.
- [x] **Seeding torrents in the Active filter.** I had the complaint backwards
      and tested the wrong thing: they were appearing and should not. Active is
      not a state, it is a question about right now, and Deluge asks it as
      "download or upload rate above zero". A seeding torrent with no peers is
      idle. The count and the filter both ask it that way now.

## Labels, finished

- [x] **The Label plugin's API.** Radarr and the rest ask
      `core.get_enabled_plugins` and will not let you set a category without
      it. The daemon reports `Label` and answers the eight `label.*` methods;
      the Web UI forwards that namespace, because those programs connect to it
      rather than to the daemon's port. `daemon.get_method_list` grows by
      exactly those methods, which is what a Deluge daemon with the plugin
      enabled advertises, and a test pins the list.
- [x] **A register of labels.** A label now exists whether or not a torrent
      carries it. Deriving the list from the torrents could never answer the
      question those programs ask, because the label they are about to use is
      the empty one.
- [x] **A Labels page**, with add, rename and remove, and the per-label rules
      behind their own switches. Rename is three existing calls rather than a
      new method: the plugin had none, and inventing one would put this
      daemon's API out of step with Deluge's.
- [x] **A Label column** in the torrent list.

## Pausing idle downloads

Asked for as "download intelligent": see what is active, pause what is not so
the queue can move, and say a minute beforehand that it is about to happen.

- [x] **The rule**, off by default, in the daemon's own five-second sweep.
      Downloads only, never the last one running, never a torrent under manual
      management, and nothing at all unless something is waiting for the place.
- [x] **The countdown in three places**: a column, the Status tab and the
      status bar. Computed in the browser from two timestamps, so it ticks
      rather than arriving stale.
- [x] **libtorrent's own rotation agrees with it.** `inactive_down_rate` and
      `inactive_up_rate` come from the same threshold the rule uses. They were
      never set at all, so the "ignore slow torrents" checkbox judged by a
      default nobody could see.

Three bugs it uncovered, none of them new:

- [x] **No pause survived a restart.** Resuming the session resumed everything
      rather than what it had stopped, and the scheduler resumes the session at
      startup. A torrent paused by hand came back running.
- [x] **Stopping at a share ratio stopped nothing** on an auto-managed torrent,
      which is the default: libtorrent's queue resumed it within the minute
      because the pause did not clear the auto-managed flag.
- [x] **A rule's pause was recorded only in the session**, not on the torrent,
      and a restart re-adds every torrent from what was recorded.

## Checkbox captions that wrapped over the next control

- [x] Ext JS pins a checkbox row to the height its config asked for and does
      not clip the label inside it, so a caption long enough to wrap was drawn
      over the control below, which had already been positioned. The row is a
      flex line now: its height is its tallest child, so there is nothing to
      spill. The thirteen fixed heights that caused it are gone. Checked at
      three window widths across every preferences page.

## Finding things in a long list

- [x] **A search box.** The `keyword` filter existed in the daemon and no
      control sent it, which is the cheapest kind of missing feature. It
      narrows the sidebar's selection rather than replacing it.
- [x] **Untick labels on the Label column header** to take them out of the
      current view, *No Label* included. Done in the browser rather than as a
      query: it is the view, it is instant, and it leaves the sidebar's
      single-label filter to mean what it means. Identifying the column needed
      its `dataIndex`, not its id: only columns that declare an id have one and
      the rest carry their position, so the first version matched nothing.

## Loose ends, whenever

- [x] **The Alpine image: dropped, with a reason.** Alpine does ship
      `libtorrent-rasterbar`, at 2.0.10 against Debian trixie's 2.0.11, which
      is the version Deluge is tested against. The saving would be roughly
      70 MB off 189 MB. Against that: a musl target for the whole build, a
      libtorrent a release behind, and musl's allocator under a threaded C++
      workload, which is exactly what this software is. Not worth it. The
      experiment is not coming back.
- [x] **Archive tools.** Not needed. The block list unpacks gzip and zip in
      process, so nothing in the image has to shell out to an archiver.
- [x] **Certificate pinning for remote daemons.** A `daemon_fingerprints`
      object in `web.conf`, host id to sha256, turns on `TlsMode::Pinned` for
      that host. The daemon prints its own fingerprint at startup, which is
      where the value comes from. Connecting to a non-loopback daemon without
      a pin warns once, rather than refusing: that would break every existing
      remote setup.
- [x] **The OpenAPI spec says nothing about return types.** It does now for 45
      of the 99, from the Python annotations the contract recorded, translated
      into JSON Schema by the generator rather than written by hand. The other
      54 either declared nothing or return a class whose fields the contract
      does not record; the specification says which, rather than guessing. The
      schemas are not referenced from the response, because one endpoint
      carries every method and OpenAPI cannot select a response by request
      body.
- [x] **Rate limiting on the web login.** A token bucket per client address:
      five attempts free, then one every ten seconds. Checked before the
      password is verified, because scrypt is deliberately slow and doing that
      work for a client already over its budget is the denial of service rather
      than the defence. A correct password clears the record.

---

## Done

Kept short, as a record of what the phases actually delivered.

- Windows and macOS support removed, Linux only. 33 200 lines.
- Docker image built from source with pinned dependencies, Web UI reachable.
- Phase 0: the contract frozen and checked automatically, the `cxx` bridge to
  libtorrent 2.0 on two architectures, resume data crossing it.
- Phase 1: rencode, the DelugeRPC framing and client, and a Web UI server that
  replaces `deluge-web` and talks to the Python daemon.
- Phase 2: the libtorrent bridge. All 40 handle methods, the 29 settings by
  name, the 24 alerts and the 40 status fields, on two architectures.
- Phase 3: the daemon. All 70 RPC methods, the 20 events, the 77 configuration
  keys, state that survives a restart, and the Rust Web UI running on it.
- Phase 4: the switchover. The Python tree deleted, a converter for existing
  installations, and a 189 MB image with no interpreter in it.
- Phase 5: the four features that were plugins. Labels on the torrent and in
  the filter tree, watched directories, the block list on libtorrent's IP
  filter through a new bridge call, and the weekly schedule. The plugin
  interface came out of the front end with them.
- Documentation: a wiki under `wiki/`, mirrored to GitHub by a workflow, with
  the phase reports archived in it and the app documentation as the main body.
  `docs/openapi.yaml` is generated from the contract and checked by the gate.
- Housekeeping: the licence header names the fork and its origin, `AUTHORS` is
  back with the bundled front end's licences, every source file carries an SPDX
  line, the changelog says where Deluge became redeluge, and the Photoshop
  sources that were being compiled into the binary are gone.
- The leftovers, all twenty of them. Four turned out to understate what was
  wrong: the upload endpoint was one of ten missing Web UI methods, the peer
  country was a missing peer list, the block list's archive tools were not
  needed at all, and gzip compression had never been switched on. Nothing is
  open.
