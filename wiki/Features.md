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

Two halves, and it is worth knowing which is which. *Which label a torrent
carries* is a torrent option, set on the torrent. *Which labels exist* is a
register kept under the `label` key of `core.conf`, because a label has to be
able to exist before anything is in it.

**In the Web UI**: Preferences, Labels manages the register, with Add, Rename
and Remove and the per-label rules. A torrent is put in a label three ways:
right-click it in the list and pick one under *Label*, which is the quickest
and works on a whole selection at once; in the Add dialog under Options; or in
the torrent's own Options tab. The Label column of the list shows which, and
the sidebar's Labels list filters on them.

The right-click menu reads the labels each time it opens rather than when the
page loaded, so a label another program has just created is already there.

**From another program**, this answers the Label plugin's own API. See
[Compatibility with Radarr, Sonarr and the rest](#compatibility-with-radarr-sonarr-and-the-rest)
below.

A label is a torrent option like any other:

```bash
core.set_torrent_options([torrent_id], {"label": "films"})
```

Labels are lower case and limited to letters, digits, `_`, `-` and `.`.
Anything else is dropped rather than refused, so `My Films!` becomes `myfilms`.
Read it back from the status key `label`, filter on it in
`core.get_torrents_status`, and see the counts in `core.get_filter_tree`, which
now carries a `label` category next to state, tracker and owner.

### What a label applies

A label can impose settings on the torrents in it, which is what the plugin's
own options did. Three groups, each behind its own switch, so a label that
names a group and changes nothing is the default rather than an accident:

| Switch | What it then applies |
|---|---|
| `apply_max` | `max_download_speed`, `max_upload_speed`, `max_connections`, `max_upload_slots`, `prioritize_first_last` |
| `apply_queue` | `is_auto_managed`, `stop_at_ratio`, `stop_ratio`, `remove_at_ratio` |
| `apply_move_completed` | `move_completed`, `move_completed_path` |

They are applied when a torrent joins the label and when the label's options
change. `auto_add` and `auto_add_trackers` are stored and reported so a client
that sets them does not lose them, and nothing acts on them yet.

### Compatibility with Radarr, Sonarr and the rest

Every program built on Deluge's API asks `core.get_enabled_plugins` whether the
Label plugin is there, and refuses to set a download category when it is not:
Radarr says *Label plugin not activated* under the Category field. This daemon
answers `["Label"]`, and answers the plugin's methods:

| Method | |
|---|---|
| `label.get_labels` | Every label, sorted |
| `label.add` | Adds one; answers `false` if it was already there rather than raising |
| `label.remove` | Removes it, and takes it off the torrents that carried it |
| `label.set_torrent` | Puts a torrent in a label |
| `label.get_options`, `label.set_options` | The rules above |
| `label.get_config`, `label.set_config` | The whole register |

They go through the Web UI's `/json` as well as the daemon's own port, which is
what these programs actually connect to.

Two deliberate differences from the plugin. `label.add` on a label that exists
answers `false` instead of raising, because clients add before every use and
swallow the error anyway. And `label.set_torrent` with a label nobody created
**creates it** rather than refusing: the add is the call most likely to have
been skipped or lost, and refusing means a download silently lands with no
category.

## Pausing idle downloads

Off by default, under Preferences, Queue. A download that holds a place in the
active queue and transfers nothing is costing another torrent its turn; this
gives that place away and gives it back later.

The rule is deliberately dull, because a rule that pauses downloads has to be
predictable:

| | |
|---|---|
| Idle below | Bytes per second under which a download counts as idle. Also what libtorrent's own "ignore slow torrents" judges by, so the two agree |
| Idle for | How long it has to stay under that rate. A torrent between pieces dips for a few seconds all the time |
| Paused for | How long it is then left alone |
| Never leave fewer running than | Without this, a queue of torrents that are all idle pauses every one of them |
| Only when a torrent is waiting | Pausing when nothing wants the place gains nothing and costs the peer that was about to turn up |

Three things it will not touch. A torrent that is seeding, because being
reachable is what seeding is. A torrent taken out of automatic management,
because that is someone running it by hand. And the last download still going,
whatever the queue looks like.

Pressing Resume on a held torrent ends the hold: the person wins. Turning the
rule off releases everything it is holding, on the next pass.

**Where you see it.** The *Idle* column of the torrent list counts down, first
to the pause and then to the release. The Status tab of a torrent says the same
thing in a sentence. The status bar shows how many torrents are being held, and
only when there are any.

The countdown is computed in the browser from two timestamps rather than being
a sentence the server wrote, so it ticks between polls instead of being as old
as the last one.

## Finding a torrent

The search box in the toolbar sends the daemon's `keyword` filter, which looks
at the name, the state, the tracker and its last message, the label and the
infohash. Every term has to match, so two words narrow rather than widen. It
applies on top of whatever the sidebar has selected, so you can search inside a
label. Escape clears it.

Separately, right-clicking the **Label** column header offers *Show labels*: a
tick per label, plus *No Label*, and unticking one takes those torrents out of
the view. That is a view filter, done in the browser, so it is instant and
survives the next poll. It answers a different question from the sidebar's
Labels list, which picks one label to look at; this one hides the ones you do
not want to see.

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
