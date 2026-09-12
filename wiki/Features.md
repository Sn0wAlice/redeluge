# Labels, watched directories, block list, schedule

These were four of Deluge's plugins. In redeluge they are part of the daemon,
so there is nothing to install and nothing to enable in a plugin manager.

That has one consequence worth stating plainly: there is no `autoadd.*`,
`blocklist.*`, `scheduler.*` or `label.*` RPC namespace, because there is no
plugin to talk to. Each feature is one key of `core.conf`, so `core.get_config`
and `core.set_config` are the whole interface and every client that speaks
DelugeRPC already has it. Labels are the exception: a label is a property of a
torrent, so it is set with `core.set_torrent_options`.

Everything below is off by default. An upgrade changes nothing until you turn
something on.

## Setting them

**In the Web UI**, under Preferences: *Watched Folders*, *Block List* and
*Schedule*. The schedule is a grid of the week; clicking an hour cycles it
through full speed, slow and stopped. The Block List page has a *Fetch Now*
button, which clears the timestamp and brings the next download forward to
within the minute. Labels have no page of their own: a label is a property of a
torrent, set in the Add dialog or in the torrent's own Options tab, and the
sidebar lists them once some exist.

**Or through the API**, which is what the pages do. Having logged in first; see
[Web API](Web-API) for the session cookie.

```bash
curl -s -b cookies.txt -H 'Content-Type: application/json' \
  -d '{"method":"core.set_config","params":[{"scheduler":{"enabled":true}}],"id":1}' \
  http://127.0.0.1:8112/json
```

Or from any Deluge client, `core.set_config({"scheduler": {...}})`.

A key is replaced whole, not merged. Read the current value, change what you
want, and send the result back.

## Labels

**In the Web UI**, a label is a text field: in the Add dialog, under Options,
and afterwards in a torrent's own Options tab. Type a name and apply. The
Labels list appears in the sidebar as soon as one torrent carries one, and
filters like any other category. There is no page in Preferences because there
is nothing global to configure.

A label is a torrent option like any other:

```bash
core.set_torrent_options([torrent_id], {"label": "films"})
```

Labels are lower case and limited to letters, digits, `_`, `-` and `.`.
Anything else is dropped rather than refused, so `My Films!` becomes `myfilms`.
Read it back from the status key `label`, filter on it in
`core.get_torrents_status`, and see the counts in `core.get_filter_tree`, which
now carries a `label` category next to state, tracker and owner.

Unlike the plugin, a label carries no options of its own. It names a group; the
per-torrent options do the rest.

## Watched directories

Torrent files dropped into a directory are added and then moved out of the way.

```json
{
  "autoadd": {
    "enabled": true,
    "interval": 5,
    "watchdirs": [
      {
        "enabled": true,
        "path": "/watch/films",
        "download_location": "/media/films",
        "label": "films",
        "add_paused": false,
        "after_add": "rename",
        "rename_extension": ".added",
        "copy_to": ""
      }
    ]
  }
}
```

`interval` is seconds between scans. `after_add` is `rename`, `leave` or
`delete`; `rename` appends `rename_extension` to the whole name, so
`a.torrent` becomes `a.torrent.added` and the next scan skips it. `copy_to`, if
set, takes a copy of the original before any of that happens.

A file is not added the moment it appears. It has to be the same size on two
scans running, because a file that is still being written parses as a corrupt
torrent and the error says nothing about why.

`.torrent` and `.magnet` files are read. A `.magnet` file is magnet links, one
per line, with blank lines and comments skipped; the file is one unit for
disposal, so a bad link among several does not leave the good ones to be added
again on every scan.

## Block list

A downloaded list of address ranges, installed as libtorrent's IP filter.

```json
{
  "blocklist": {
    "enabled": true,
    "url": "https://example.invalid/list.p2p.gz",
    "check_after_days": 4,
    "timeout": 180,
    "try_times": 3,
    "whitelisted": ["10.0.0.0 - 10.255.255.255", "192.168.1.5"]
  }
}
```

The list is downloaded when the cached copy is older than `check_after_days`,
and the cache is what a restart loads, so a daemon that starts without a
network still filters. Set `check_after_days` to zero to pin a list you
downloaded yourself. The settings are read every minute, so changing the URL
fetches the new list at once, changing the whitelist reinstalls from the cached
one without downloading anything, and turning `enabled` off clears the filter.

Two formats are read, both text, and which one is in use is detected from the
first line that says anything:

| Format | A line looks like |
|---|---|
| PeerGuardian, also called SafePeer or p2p | `Some organisation:1.2.3.4-5.6.7.8` |
| eMule | `001.002.003.004 - 005.006.007.008 , 000 , Some organisation` |

Plain, gzipped and zipped lists are read. A zip is unpacked by taking its
largest member, because these archives often carry a readme beside the list. A
bzip2 archive is refused with a message naming the format: it would mean a C
dependency for something no list actually uses. Lines that will not parse are
counted and skipped, because a public list of two hundred thousand lines
usually has a few; the count goes in the log next to the number of ranges
installed.

`whitelisted` entries are never blocked whatever the list says. They are
applied after the list, as allowing rules, which is what puts a hole in a
blocked range.

Two keys are written by the daemon rather than by you: `last_update` and
`list_size`.

## Schedule

What may run, by hour of the week.

```json
{
  "scheduler": {
    "enabled": true,
    "button_state": [[0, 0, 0, 0, 0, 0, 0], "... 24 rows in total"],
    "low_down": 50.0,
    "low_up": 20.0,
    "low_active": 4,
    "low_active_down": 2,
    "low_active_up": 2
  }
}
```

`button_state` is 24 rows of 7, indexed `[hour][weekday]` with Monday as
weekday 0. This is the shape the plugin stored, so a `button_state` out of an
existing `scheduler.conf` can be pasted in and means the same thing. Each cell
is one of:

| Value | Effect |
|---|---|
| 0 | The configuration's own limits apply |
| 1 | The `low_*` limits apply |
| 2 | The session is paused |

The `low_*` rates are in KiB/s, and -1 is no limit, as everywhere else in
Deluge. The schedule is evaluated on the hour in local time, so a rule written
for 9am still means 9am after a clock change, and also every minute in between,
so a grid you have just edited means something before the hour is out. Set the
container's timezone with `TZ` if it is not UTC.

A cell the grid does not have is read as no restriction, so a hand-edited grid
that is too short does not stop everything.
