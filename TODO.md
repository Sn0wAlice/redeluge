# TODO

Everything known to be outstanding, from the phases done so far. The roadmap
says what the phases are; this says what is actually left.

Items are grouped by when they have to be done, not by size. Anything marked
**blocking** stops a later phase from being correct.

---

## Phase 1 leftovers

These make the Rust Web UI a complete replacement rather than a working one.

- [ ] **Torrent upload.** `POST /upload`, which the add-by-file dialog posts to.
      Adding by magnet and by URL already work, because those are daemon calls,
      so this is the only way the interface is still short of the Python one.
      The shape it needs: accept a multipart upload, write each file to a
      temporary path, and return those paths, which the front end then passes
      to `web.add_torrents`. The Python implementation that did this is gone,
      so the reference now is the front end in
      `crates/redeluge-web/assets/js/deluge-all/add/`.
- [ ] **Minified script bundles.** `build.rs` concatenates but does not minify,
      so the browser downloads roughly twice what it needs. `ScriptSet` already
      picks by what is present, so producing the minified files is the whole
      change. Check a minifier against ExtJS-era JavaScript before trusting it.
- [ ] **An integration test that boots the server.** The web crate's tests cover
      the pure logic. Nothing exercises the HTTP surface, so the JSON dispatch,
      the session cookie and the asset routes are checked by hand today.

## Phase 3 leftovers

All are behaviour the Python daemon has and this one does not yet.

- [ ] **The Web UI does not reconnect to a restarted daemon.** It connects once
      at startup and reports "connection lost" until it is restarted itself.
- [ ] **Move on completion.** The option is stored and reported; nothing moves a
      torrent when it finishes.
- [ ] **Stop and remove at ratio.** Stored, reported, and used for the seeding
      countdown, but no rule enforces them.
- [ ] **Progress while creating a torrent.** `core.create_torrent` works but
      sends no `CreateTorrentProgressEvent`, so a client shows a frozen dialog.
- [ ] **Filtering is exact-match only.** Enough for the state, tracker and owner
      filters every client sends; anything else returns nothing.

## Phase 2 leftovers

Small, and all of them belong to the daemon rather than to the bridge.

- [ ] **Peer country.** `PeerInfo::country` is always empty. It is a GeoIP
      lookup Deluge does itself, not something libtorrent reports, so it
      belongs in the daemon.
- [ ] **An SSL torrent, end to end.** The certificate call is wired and refuses
      an unknown torrent correctly, but exercising it needs an SSL torrent and a
      certificate authority.

## Phase 5 leftovers

The four features landed. These are the edges of them.

- [ ] **No Web UI for any of the three settings.** Labels appear in the sidebar
      and on the torrent, because the filter tree and the status carry them and
      the front end draws those generically. Watched directories, the block
      list and the schedule are configured through `core.set_config` and have
      no preferences page. Each needs one, and the schedule needs a grid
      widget, which is most of the work.
- [ ] **Magnet files in a watched directory.** The plugin also read `.magnet`
      files, one link per line. Only `.torrent` is read here.
- [ ] **Zip and bzip2 block lists.** Gzip and plain text are read; the other
      two are detected and refused by name. Both would be another dependency,
      and the lists people actually use are gzipped.
- [ ] **The block list is not re-read when its URL changes.** It is fetched
      when the cached copy is stale, so changing the URL takes effect on the
      next check rather than at once. Setting `check_after_days` to 1 and
      waiting an hour is the workaround.
- [ ] **A schedule change applies on the hour.** Turning the schedule on does
      not take effect until the next hour boundary, because that is when the
      state is next computed.

## Loose ends, whenever

- [ ] **The Alpine image was never verified running.** It was measured against
      the Python image, which no longer exists; the Rust one is 189 MB on
      Debian, so the saving would be smaller now. Either finish the check or
      drop the idea.
- [ ] **Archive tools.** If the block list work needs to read compressed lists,
      `7zip` and `unrar-free` are not in the runtime image.
- [ ] **Certificate pinning for remote daemons.** `TlsMode::Pinned` exists and
      nothing uses it. The Python client verifies nothing, which is defensible
      over loopback and not across a network.
- [ ] **The OpenAPI spec says nothing about return types.** The contract records
      each method's parameters, not the shape of what it answers, so a generated
      client handles the envelope and hands back an untyped value. Filling that
      in means describing 99 return shapes by hand, which would drift.
- [ ] **Rate limiting on the web login.** Neither implementation has any. The
      password is now scrypt, so an online guess is slow, but slow is not none.

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
