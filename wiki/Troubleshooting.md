# Troubleshooting

Both binaries log to standard output, at `info` by default. Raise it with
`RUST_LOG`:

```bash
RUST_LOG=debug redeluged
RUST_LOG=redeluge_daemon::features=debug redeluged
```

In the container, `DELUGE_LOGLEVEL` sets it for both processes and
`docker compose logs -f` shows them.

## The daemon will not start

**"refusing to start: the torrent list has not been converted"**

An existing Deluge installation whose `torrents.state` is still a pickle. Run
the converter; see [Migrating from Deluge](Migrating-from-Deluge). Starting
with an empty list would look exactly like having lost everything, which is why
this is a refusal rather than a warning.

**"could not bind"**

Something already has port 58846. Another daemon, usually. `ss -lntp` says
what.

**The certificate**

If the log says the daemon certificate was unusable and has been regenerated,
that is expected on a first start against a Deluge configuration: Deluge wrote
X.509 version 1 and rustls will not use it. The old files are beside the new
ones with `.unusable` appended.

## The Web UI says "connection lost"

It reconnects on its own, backing off from five seconds to a minute, so a
daemon that has just restarted is picked up within a few seconds and one that
is down for an hour is not hammered. The log says `reconnected to the daemon`.
If it never comes back, the daemon is not listening: check `docker compose
logs` or `journalctl -u redeluged`.

## The Web UI is blank, or unstyled

A blank page with JavaScript errors in the console means the script bundle is
wrong, not missing. Check that the build actually bundled the front end:

```bash
strings target/release/redeluge-web | grep -c deluge-all
```

Zero means the asset archive came out empty, which is a build problem rather
than a runtime one. The container build asserts this, so it cannot ship broken.

If labels in the interface read `${escape(_("..."))}`, an unrendered template
is being served as a static asset. That bug is gone with translation, and a
test asserts no shipped asset carries the marker, so this should be
unreachable.

## Changing the theme breaks the interface

Fixed, but a configuration written before the fix still carries the damage.
`web.get_themes` used to answer with a flat list of names where the interface
expected name and label pairs, so ExtJS read each name as a row and took its
first character as the value: choosing `gray` stored `g`. The page then asked
for `themes/css/xtheme-g.css`, which does not exist.

The page now falls back to the default theme when the stored one has no
stylesheet, and says so in the log. To clear it properly, pick a theme again in
Preferences, or fix `web.conf` while the server is stopped:

```json
{"theme": "gray"}
```

Three themes ship: `gray`, `blue` and `access`.

## Cannot log in

The password is whatever `DELUGE_WEB_PASSWORD` was on the first boot, or
Deluge's default of `deluge` if it was never set. To change it in the
container, set `DELUGE_WEB_PASSWORD` and `DELUGE_WEB_PASSWORD_RESET=1` for one
run.

Wrong passwords are rate limited: five from one address, then one every thirty
seconds. The password is also scrypt, so each guess costs real time. Neither is
a reason to expose the Web UI without something in front of it.

## A torrent will not add

**From a watched directory.** The scan reads `.torrent` and `.magnet`, only at
the top level of the directory, and only once the file has been the same size
on two consecutive scans. A file still being written is deliberately ignored. Turn the
log up and look for `could not add a torrent from a watched directory`, which
carries the reason.

If the file was added but stayed in the directory, the log says
`could not dispose of a torrent file after adding it` and the next scan will
try it again and be refused as a duplicate. Usually a permissions problem on
the directory.

**By URL.** The daemon fetches it, so the daemon needs to reach the host, not
your browser.

## The block list is not blocking

Look for `installed the block list` in the log. It carries the format detected,
the number of ranges, and how many lines were skipped.

| Log line | Meaning |
|---|---|
| `no reader recognises this list` | Neither text format matched the first line that says anything |
| `bzip2 lists are not supported` | Plain, gzip and zip are read |
| `could not download the block list` | With the URL and the reason; it retries `try_times` |
| a high `skipped` count | The file is probably the other format, or not a list |

Nothing happens at all if `enabled` is false, or if the cached copy is not yet
stale and there is no cache. Changing the URL fetches the new list within the
minute, so there is no need to force a refetch.

## The schedule did nothing

It is evaluated on the hour and every minute in between, so a grid you have
just edited takes effect within the minute. Look for `the schedule changed` in
the log, which names the state.

It is in local time. In a container that means UTC unless you set `TZ`.

The grid is `[hour][weekday]` with Monday as weekday 0. Indexed the other way
round it applies Tuesday's rules on Wednesday, silently.

## Something is set and does nothing

A control that was there before an upgrade and is gone now was removed because
nothing acted on it. [Configuration](Configuration) lists all of them with the
reason. The settings themselves still exist in the API, so a client that reads
them keeps working.

Two things need something you have to provide:

- **Peer countries are empty** unless `geoip_db_location` names a MaxMind DB
  country database. None is shipped, because its licence does not allow it.
  The legacy `GeoIP.dat` Deluge defaulted to is retired and the log says so.
  The flags themselves are shipped and are served from `/flag/<code>`; a
  country the flag set does not cover shows nothing rather than a broken
  image.
- **A remote daemon's certificate is not verified** unless you pin it. See
  [Configuration](Configuration).

## Logging in is refused after a few tries

Five wrong passwords from one address, then one attempt every thirty seconds.
The message says how long to wait. A correct password clears the record at
once, and a correct password is never held up however many tabs you open.

## The interface makes too many requests

It polls every two seconds by default, and that is one call for the torrent
list plus one for the open details tab. To slow it down, change *Refresh (ms)*
under Preferences, Interface, or set `poll_interval` in `web.conf` directly. It
is clamped between 500 and 60000, and a change takes effect on the next poll
without a reload.

```json
{"poll_interval": 5000}
```

Before that setting existed, every filter click and menu action started a poll
on top of the scheduled one, so a burst of clicking produced a burst of
overlapping requests. A poll now replaces the pending one and will not start
while another is still in flight.

## After an upgrade the interface behaves oddly

Assets are cached for an hour, so a browser could keep the previous build's
JavaScript and run it against the new server. Asset URLs now carry the build's
identity, so an upgrade changes them and the cache is bypassed. If you are
looking at a page loaded before that change, reload it once while ignoring the
cache.

## Rows in the torrent list are empty

Fixed. The interface renders only the rows that are on screen, and it used to
decide which those are from a fixed row height. Anything that changed the real
height, a browser zoom other than 100%, a larger default font, a theme with
more padding, moved that window off the bottom of the viewport, and a long
enough list moved it far enough to blank every row in view. The height is
measured now.

If rows are still empty after an upgrade, the browser is running the previous
build's JavaScript; reload once while ignoring the cache.

## A preferences page ends with nothing below it

Fixed. Pages scroll now, and the window can be resized. Before that the card
layout sized the active page to the window and anything taller was simply cut
off, with no scrollbar and nothing to say there was more: the Bandwidth page
lost its per-torrent limits that way, and the Schedule page lost everything
below the grid.

## The Files tab of a torrent is empty

Fixed. The daemon did not report `files`, `file_progress` or `file_priorities`
in the torrent status at all, so the tree the interface builds from them was
always empty. They are reported now, and only fetched when a client asks for
them, because each is a separate call into libtorrent.

A torrent added from a magnet still shows nothing until its metadata arrives:
until then it has no file list to report.

## The tracker column has no icons

By design. Deluge fetched each tracker's favicon through its web server and
cached it. That means the server making a request to every host your torrents
name, so there is no such fetcher here and no `/tracker/<host>` route. The
column is the host name.

## The Labels list in the sidebar is empty

It only appears once a torrent has a label. Set one in the Add dialog under
Options, or afterwards in the torrent's own Options tab, and apply. Before
that field existed nothing in the interface could set a label, so the list was
empty however many torrents there were.

## The block list will not refresh

Press *Fetch Now* on the Block List preferences page. It clears the stored
timestamp, and the next check, within the minute, treats the list as one that
has never been fetched. There is no method for forcing a download, on purpose:
the daemon answers Deluge's API and nothing besides.

## Reporting a problem

Include the version line from the daemon's first log line, which carries both
the reported version and the libtorrent it linked against, and say whether you
came from a Python Deluge installation.
