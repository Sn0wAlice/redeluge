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

**In the Web UI**: Preferences, Labels manages the register, with Add, Edit,
Rename and Remove. What a label *applies* opens in a window of its own, from
*Edit* there or by right-clicking the label in the sidebar and choosing
*Settings*, so reading what a label does is no longer the same gesture as
selecting it to be written back. A torrent is put in a label three ways:
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
own options did. They are set in the label's own window — from *Edit* in
Preferences, Labels, or by right-clicking the label in the sidebar. Three
groups, each behind its own switch, so a label that names a group and changes
nothing is the default rather than an accident:

| Switch | What it then applies |
|---|---|
| `apply_max` | `max_download_speed`, `max_upload_speed`, `max_connections`, `max_upload_slots`, `prioritize_first_last` |
| `apply_queue` | `is_auto_managed`, `stop_at_ratio`, `stop_ratio`, `remove_at_ratio` |
| `apply_move_completed` | `move_completed`, `move_completed_path` |
| `apply_stuck` | `stuck_hours`, `stuck_remove_data` — see below; it is the one that deletes |

The first three are applied when a torrent joins the label and when the label's
options change. The fourth is not a torrent option at all: it is something the
daemon does on a sweep, once a minute, and it is described on its own below. `auto_add` and `auto_add_trackers` are stored and reported so a client
that sets them does not lose them, and nothing acts on them yet.

### Hiding a label by default

One more option, under *In the torrent list*, that does nothing to the torrents
at all: `hide_by_default` keeps them out of the list until you ask for them.

It is the same thing as unticking the label under *Show labels* in the Label
column's header menu, except that it starts that way, and it is stored with the
label rather than in one browser: the machine you open tomorrow hides it too.
Useful once a label holds a few hundred torrents you never look at and the rest
of the list is what you actually manage.

Nothing is lost and nothing is stopped. Picking the label in the sidebar shows
those torrents, because asking for a label outranks hiding it. Ticking it in
the header menu shows it for the rest of the session. The sidebar's count still
counts them, every other client still sees them, and the daemon still runs
them.

### Throwing away downloads that never start

One more option, under *Downloads that never start*, and the only one here that
deletes anything: `apply_stuck` removes a torrent of this label that has been
trying for `stuck_hours` and has not downloaded a single byte.

A torrent in that state is not slow, it is dead — a magnet nobody is seeding, a
`.torrent` for content that has left the swarm, a tracker that has stopped
answering for it. In the list it looks exactly like one that is merely between
peers, and the only way to tell them apart is to remember when it was added.

The delay is counted in **time the torrent spent trying**, not on the wall
clock. A torrent that sat in the queue for a day, or that was paused over the
weekend, has not been failing for a day: libtorrent's `active_time` is the
clock, and it survives a restart.

It is narrow on purpose. It will not take:

| | |
|---|---|
| A torrent that downloaded **anything at all** | One byte is the difference between a dead swarm and a bad week |
| A **paused** one, including one the queue is holding back | Somebody paused it, or the daemon did, and neither is the torrent failing |
| A **finished** one | A torrent with every file deselected is finished at zero bytes, on purpose |
| One **checking** or **moving** | Neither has had a chance to download, and removing a torrent out from under libtorrent's file handling is how half of it ends up in each place |

`stuck_remove_data` deletes the files too, and unlike the tracker rule's
equivalent it is **on by default**. The difference is the point: that rule
removes torrents that finished, where the files are the whole reason they
exist; this one removes torrents that downloaded nothing, where "its files" is
an empty directory and whatever was preallocated. Leaving those behind is how a
download directory fills with the skeletons of torrents that never ran.

`stuck_hours` of zero is a rule, not an off switch: it means the next sweep
takes anything that has done nothing since it started. The switch is what
decides. Removals are recorded in *Activity* under the **Label** rule, with the
label and the delay that took it.

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

## Tracker rules

Right-click a tracker in the sidebar and choose *Settings*. A tracker is not
something you create, the way a label is: it is whatever the torrents you added
announce to, and the sidebar has been grouping them by it all along. This is a
place to say what should happen to that group, once, instead of setting it on
every torrent that arrives from it.

Four rules, each off for every tracker until you turn it on for one by name.
They are applied in the order below, which is the order that matters when a
torrent qualifies for more than one: a torrent is held to its limits and filed
under the right name before it is moved, and moved before it is taken away.

### Limit them

| Option | |
|---|---|
| `auto_limit` | Hold this tracker's torrents to the limits below |
| `max_download_speed`, `max_upload_speed` | KiB/s, `-1` for no limit |
| `max_connections`, `max_upload_slots` | `-1` for no limit |

The same four numbers a label carries, applied the same way: through
`core.set_torrent_options`, exactly as if you had set them on each torrent.

They are applied when a torrent from this tracker is first seen, and again
whenever you change them — not on every pass. A standing rule that overwrote a
limit you set by hand, once a minute, would be unusable; this way the rule
gives way until you move it. After a restart every torrent is seen for the
first time again, so a rule you have not changed writes the same numbers back.

### Label them

| Option | |
|---|---|
| `auto_label` | Put these torrents in a label |
| `label` | Which label. It is created if it does not exist, and whatever that label applies is applied |
| `label_on_add` | When the torrent arrives, if it has no label yet. On by default: a rule that names a label and says nothing about when means the obvious thing |
| `label_when_done` | When it has finished, replacing whatever label it has |
| `label_after_hours` | How long to wait after it finished, for that second one |

Arrival never overwrites: a torrent you filed by hand stays where you put it.
Completion does, because that is what it is for — moving a torrent from the
label it downloaded under to the one it is kept under.

### Move them

| Option | |
|---|---|
| `auto_move` | Move the files once the torrent has finished |
| `move_path` | Where to. A move to where they already are is not a move |
| `move_after_hours` | How long to wait after it finished |

The destination disk is measured before anything is copied, and it is not
always the disk the files are on now: a folder inside the download folder can
be a mount point for another drive, which is exactly the case that fills a disk
up. If the move would leave under a gibibyte free at the destination, it is not
started, the log says so once, and it is reconsidered on the next pass — so
freeing space is all it takes. A move within one filesystem is a rename that
writes nothing, and is never refused.

### Remove them

| Option | |
|---|---|
| `auto_remove` | Remove a finished torrent once its wait is up |
| `remove_after_hours` | How long to wait after it finished downloading. `0` means the next sweep, about a minute |
| `remove_data` | Delete the downloaded files as well. Off, so the rule as first turned on removes the torrent and leaves the download alone |

A tracker that asks for a day of seeding can be given a day and then clean up
after itself, while the tracker beside it in the list is not touched. The
removal is the same removal `core.remove_torrent` performs, announced to every
connected client the same way, so an interface showing the torrent lets go of
it rather than holding a row that no longer exists.

### What "finished" means, and the one case it does not cover

Every wait is measured from the moment the download finished, which is the time
libtorrent recorded and keeps in the resume data, so it survives a restart of
the daemon. Nothing is ever considered before the torrent is finished, and a
torrent still being moved is left alone until it lands.

libtorrent has no such time for a torrent it never saw finish, which is what
adding a torrent over files that were already on disk looks like. The two
kinds of rule answer that differently, on purpose:

- **Labelling and moving** fall back to the time the torrent was added. Both
  are undoable, and a rule that silently skipped every imported torrent would
  be a rule that looks broken.
- **Removing** does not. Deleting on a wait measured from a time nobody knows
  is the one way this could take something unexpectedly, so a torrent with no
  completion time is never removed by a tracker's rule.

The daemon looks over the library once a minute.

### Seeing it coming

The torrent list has a *Tracker Rule* column, beside *Idle*, and it says what
is about to happen to each row: *removed in 21h*, *moved in 2h*. Removal is
named first when both are due, because it is the one that cannot be undone.

It is counted in the browser from two timestamps the status carries,
`tracker_remove_at` and `tracker_move_at`, so it ticks between polls rather
than being as old as the last one. Zero means nothing is coming and the column
is blank — which is what it says for every torrent until you set a rule. The
conditions behind those two numbers are the sweep's own: a countdown that
reaches zero and is followed by nothing would be worse than no countdown.

### Before you arm one

The window says what the form in front of you would do, under the buttons, and
it says it again every time you change a field:

> 3 torrents here, 2 of them finished.
> 2 would be removed with their files, freeing 420 MiB, the first in 21h.

It is an estimate and phrased as one — the daemon is what acts — but it is the
same arithmetic on the same numbers, so it is the difference between arming a
rule and discovering what it meant an hour later. A rule that would act on the
next sweep says so in those words rather than counting down from nothing.

### Setting them without the interface

It is one key, like the rest:

```bash
curl -s -b cookies.txt -H 'Content-Type: application/json' \
  -d '{"method":"core.set_config","params":[{"tracker":{"trackers":{
        "example.org":{"auto_remove":true,"remove_after_hours":48,"remove_data":true,
                       "auto_move":true,"move_path":"/archive","move_after_hours":1,
                       "auto_label":true,"label":"films","label_when_done":true}}}}],"id":1}' \
  http://127.0.0.1:8112/json
```

The key is the tracker host as the sidebar groups it — `example.org`, not the
announce URL — because that is the row the rule is set on. Remember that a key
is replaced whole: send every tracker you want to keep a rule for, not just the
one you are changing. The Web UI reads the current value and merges for you.

## Which trackers are down

Right-click a tracker in the sidebar and choose *Info*.

A sidebar row is a domain, not a tracker. `tracker.example.org` and
`backup.example.org` are grouped into one row, because that is the unit a set
of rules belongs to. What they do not share is a state: one can be refusing
every announce while the other answers, and the only trace of that used to be
the tracker column of whichever torrent happened to be on the failing one. A
tracker down for a week looked exactly like a torrent nobody is seeding.

The window takes the row apart again. A *Summary* tab for the domain, then one
tab per announce URL under it, each of them saying:

| | |
|---|---|
| Whether it answers | *Working*, *Trouble*, *Down* or *Not contacted yet*, with the tracker's own error message under it, verbatim |
| Torrents | How many list this tracker, and how many are announcing to it rather than to one of its siblings |
| Swarm | Seeds and peers, from the announces this tracker actually answered |
| Size, downloaded, uploaded, ratio | What its torrents add up to — the ratio being the number most trackers ask about |
| Next announce | When the soonest one is due |

The sidebar carries the same answer as a colour: a dot on each tracker row, in
the indent the tracker's favicon used to occupy.

| | |
|---|---|
| Green | Announces are getting through |
| Amber | Some are failing, or the tracker has stopped recognising torrents |
| Red | Every announce to this domain is failing |
| Grey | Nothing has been tried yet |

*Down* is deliberately the strong word. One failing announce among working ones
is trouble, not a tracker that has gone away, and a colour that cries wolf over
a single stale announce is one you learn to ignore.

Nothing here announces or scrapes. Every figure is one libtorrent was already
holding, because opening a window is not a reason to send a tracker several
hundred requests — which is also why BEP 48's full scrape is switched off
nearly everywhere it was ever offered.

Over the API there are two methods. The detail of one domain, keyed the way the
sidebar keys it:

```bash
curl -s -b cookies.txt -H 'Content-Type: application/json' \
  -d '{"method":"redeluge.get_tracker_info","params":["example.org"],"id":1}' \
  http://127.0.0.1:8112/json
```

And the colours, one word per domain, which is all the sidebar needs:

```bash
curl -s -b cookies.txt -H 'Content-Type: application/json' \
  -d '{"method":"redeluge.get_tracker_health","params":[],"id":1}' \
  http://127.0.0.1:8112/json
```

The torrents counted are the ones the sidebar counts — the ones announcing to
this domain — so the total in the window and the number on the row agree.
Within them, a torrent counts towards every tracker of the domain it lists, not
only the one it is announcing to: a listed backup that has been failing for a
month is exactly what this is for.

## Torrents the tracker has dropped

A private tracker that has pruned a torrent answers every announce the same
way for ever: *Unregistered torrent*, *Torrent not found*, *not registered with
this tracker*. The torrent then sits in the list seeding to nobody, counting
for nothing, and at a glance it looks exactly like one that simply has no peers
today.

The sidebar separates them. Under *States*, beside *Active*, there is
**Unregistered**: every torrent whose tracker last said it had no record of it.
Right-click that row and *Remove these torrents...* hands the whole group to
the ordinary Remove dialog, which asks — as it always has — whether to keep the
files or delete them. Nothing here removes anything on its own.

It is deliberately narrow. A tracker that is down, refusing connections or
rate-limiting is *not* a tracker that has forgotten the torrent, and a check
that lumped the two together would offer to delete a library because a tracker
was rebooting. Only the phrases that mean "I have no record of this" count:
`unregistered`, `not registered`, `torrent not found`, `unknown torrent`,
`info hash not found`. Everything else stays an error like any other.

Like *Active*, it is a question rather than a state: the torrents in it are
still Seeding or Downloading as far as libtorrent is concerned, and no other
client sees anything unusual about them. Over the API it is a filter like the
rest — `core.get_torrents_status({"state": "Unregistered"}, ["name"])` — and
`core.get_filter_tree` counts it beside the states.

## What each peer has done

Off by default, turned on from the **Peers** window in the toolbar.

The Peers *tab* shows the connections open right now, at the speeds of the
moment. That cannot answer the question people actually have about an address:
*what has it ever given back?* A peer that takes forty gibibytes over a week
and sends nothing looks idle in every snapshot, because it is idle in every
snapshot. libtorrent's own counters belong to a connection and die with it.

So the daemon keeps a running account, by address, and the window reads it:

| Column | |
|---|---|
| Took / Gave | Bytes this daemon sent to the address, and bytes it sent back |
| Gave back | The second over the first. A dash when it never took anything — "asked for nothing" and "asked and gave nothing" are different facts |
| Torrents | How many of your torrents it has been seen in. Pick the row and the panel below names them |
| Cross-seeds | How many of your *contents* it carries on more than one of your torrents |

Totals are accumulated by difference, never copied: libtorrent's per-connection
counters reset when a peer reconnects, so a sample smaller than the last one is
a new connection and the whole of it is new. Sampled every fifteen seconds, and
only for torrents that have peers.

Sampling has one blind spot worth knowing about: a connection that begins and
ends between two samples is not counted at all. Nothing reports a peer's totals
as it disconnects, so there is no other mechanism available. The bias is in the
harmless direction — a peer too brief to be sampled is a peer too brief to have
taken anything worth the name, and the ones this exists to find are the ones
that stay for hours. Over a local link, where a whole file can move in under a
second, it misses nearly everything; that is a fact about local links, not
about swarms.

### About that Cross-seeds column

It is not an accusation, and it is worth saying so plainly. A peer seeding the
same release to two trackers uploads real bytes to both swarms; the ratio it
earns on each is ratio it actually earned. Most private trackers allow it, and
it is exactly what this daemon's own tracker rules help you do. The column is
there because it is interesting, and because it is the quickest way to confirm
that your own cross-seeding is working.

It is also worth knowing what it cannot see. Announcing a fake upload figure to
a tracker — the actual way ratios are cheated — is invisible from inside a
swarm: the client doing it never talks to you. That detection belongs to the
tracker, which can compare what one member claims to have uploaded against what
everybody else claims to have downloaded. Nothing here can do it.

And an address is not a person: a VPN exit, a seedbox or a carrier-grade NAT
puts many people behind one.

### What it keeps

The last thirty days by default, changed in the window itself, and at most
twenty thousand addresses — the oldest go first when that is reached. Kept in
`state/peers.json` beside the torrent list, so it survives a restart; the
per-connection counters are dropped on the way out, because they describe
connections that will not exist next time.

It is a record of what this machine saw on its own link. Nothing about it is
sent anywhere, and no rule in this daemon acts on it.

A torrent removed since is still listed, by its infohash: what a peer moved is
no less true for the torrent being gone.

The toolbar has a box that narrows the list by address or client, and every
column sorts. The search is answered by the daemon, not applied to what is on
screen: the window holds the five hundred biggest takers, and the peer you are
looking for is usually not one of those.

Over the API: `redeluge.get_peers`, biggest taker first, optionally with a
limit and a search to narrow by address or client before that limit. Each entry's `torrents` is the list itself — hash, name and whether that
torrent is one the peer also carries elsewhere — not a count. The `peers` key of `core.conf` holds `enabled` and `ttl_days`.

## What the daemon did on its own

Seven things here act without being asked: the share-ratio rule, the idle rule,
the disk-space rule, the schedule, a tracker's rules for labelling, moving and
removing, and a label's rule for downloads that never start. Three of them move
or delete files.

**Activity**, in the toolbar, is the short history of what they did — newest
first, with the torrent, the rule and why. It is not a log viewer: it answers
the two questions somebody actually asks once automatic rules are on, *why is
that paused* and *what happened to that download*, and it answers them beside
the torrents rather than in `docker logs`.

| | |
|---|---|
| Tracker | Labelled, moved or removed by a tracker's rule |
| Label | Removed by a label's rule for downloads that never start |
| Idle | Paused for transferring nothing while something was queued, or let go again |
| Disk | Paused because the disk it writes to is nearly full, or let go once there was room |
| Ratio | Stopped or removed at its share ratio |
| Schedule | The weekly schedule changed what is allowed to run |

The last two hundred are kept, in memory. That is deliberate: the history
explains the state the daemon is in now, which a restart clears anyway, and a
file would need rotating, locking and a format for something read twice a
month. The daemon's log still has every line, and keeps them.

Over the API it is `redeluge.get_recent_actions`, optionally with a limit:

```bash
curl -s -b cookies.txt -H 'Content-Type: application/json' \
  -d '{"method":"redeluge.get_recent_actions","params":[20],"id":1}' \
  http://127.0.0.1:8112/json
```

Each entry is `time`, `rule`, `did`, `torrent_id`, `name` and `detail`. The
name is stored with the entry rather than looked up, because after a removal
there is nothing left to look it up in. `redeluge.*` is this fork's own
namespace, kept separate from `core.*` so that nothing can mistake one for a
Deluge method; it is the only one, and it is advertised in
`daemon.get_method_list` like everything else.

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

## Telling you when a torrent finishes

Off by default, under Preferences, Notifications. A POST goes out when a
download finishes, when a torrent goes into error, and optionally when one is
added.

This is what most people installed the Execute plugin for. Execute did it by
running a shell script as the daemon user with the torrent's name as an
argument, which is a remote code execution primitive wearing a convenience hat.
A POST is the same result and runs nothing.

Four kinds of destination, as many of each as you like:

| | |
|---|---|
| Discord | The incoming-webhook URL from the channel's settings. The message arrives as an embed, green when a download finished and red when something broke |
| ntfy | The topic URL you would open in the app. The token field takes an access token for a protected topic |
| Gotify | The server URL, plus an application token. `/message` is added for you, and a token already in the URL is left alone |
| Webhook | One JSON object posted to anything else: the event, the torrent's name, size, path, label, tracker, ratio and error. The token, if you set one, is sent as a bearer token |

Every one of them is a POST with a JSON body. That is not only tidiness: ntfy's
header form cannot carry anything outside ASCII, and a good half of the torrent
names that matter are not ASCII.

**Send test** posts a sample message to every destination on the page and
writes back what happened, under the grid. A wrong URL says so there rather
than in a log nobody is watching. A destination that answers with a refusal is
not retried; one that times out or fails is, three times.

Two things it will not do. It does not announce your whole library when the
daemon restarts: libtorrent reports a torrent as finished again after it
re-checks one that was already complete, so a completion older than five
minutes is not news. And it does not announce the torrents restored at startup,
only the ones that arrive while it is running.

## Running out of disk space

On by default, under Preferences, Downloads. Every other feature here waits to
be asked; this one does not, because the failure it prevents happens once and
then takes an evening to clean up.

What happens without it is worth spelling out. The disk fills, libtorrent fails
a write, that torrent goes to Error, and the next one does the same a minute
later. Nothing resumes on its own once there is room again: you free some space
and then restart thirty torrents by hand, having first worked out which of them
stopped for this reason and which for another.

So: under **1 GiB free**, every torrent still writing to that disk is paused.
Over **2 GiB free**, the ones it paused are started again. Both numbers are
yours to change.

| | |
|---|---|
| Pause below | Free space at which downloads stop. 1 GiB by default |
| Resume above | Free space at which they start again. Higher than the floor on purpose: one threshold would pause, release and pause again as pieces are discarded |

Three things it will not touch. A torrent that is seeding, because a seed
writes nothing and cannot be the reason a disk filled. A torrent already paused
by you, by the queue's ratio rule or by the idle rule, which is already stopped
and stays stopped. And a torrent on a disk that still has room, because
stopping it would fix nothing: the rule reads each filesystem the session is
writing to separately.

It also remembers whether the queue was managing a torrent before it took it,
and gives that back on release. A torrent you were running by hand does not
come back under the queue because a disk filled up.

A path it cannot measure at all, an unplugged disk or an unmounted share,
decides nothing either way. Stopping everything whenever a mount blinks would
be worse than the problem, and starting downloads again with no idea whether
there is room would be worse still.

**Where you see it.** A counter in the status bar, beside the idle rule's own
and hidden the same way when it is holding nothing; clicking it opens these
settings. The Status tab of a torrent says whether this is what stopped it.
Pressing Resume yourself ends the hold, though if the disk is still full the
rule takes it again within fifteen seconds, which is the one case where a rule
should win: there is nowhere to put what it would download.

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
not want to see. A label can also start unticked, for everybody, which is
[hiding a label by default](#hiding-a-label-by-default) above.

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
