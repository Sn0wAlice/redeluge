# Phase 5 report: the four features

> **Archived.** Part of the Rust migration, which is finished.
> See [Migration Overview](Migration-Overview) for the whole story, and
> the pages under **Using redeluge** for how the software works today.

**Status: done.** Labels, watched directories, the block list and the weekly
schedule are part of the daemon. The plugin interface is out of the Web UI.
There is nothing left to install and no plugin namespace to call.

## The decision that shaped all four

The plugins had RPC namespaces of their own: `autoadd.set_options`,
`blocklist.get_status`, `label.set_torrent`. None of those is in the frozen
contract, because the contract was extracted from the daemon and the plugins
were not part of it.

So there were two ways to go. Add the namespaces back, and invent a wire
surface that no client knows and nothing checks. Or put each feature's
settings in `core.conf` under one key, where `core.get_config` and
`core.set_config` already carry anything, and where every client that speaks
DelugeRPC can reach them today.

The second. It adds no method, changes no contract, and needs no client to
learn anything. The cost is that the settings are a JSON dictionary rather than
a typed call, which is why each feature parses its key defensively: a client
can write whatever it likes there, and a daemon that refuses to start because
someone sent a string would be worse than one that schedules nothing.

Labels are the exception, and for the same reason. A label is a property of a
torrent, so it is a torrent option: `core.set_torrent_options` already takes an
arbitrary dictionary and already exists.

## Labels

The phase 3 report and the TODO both said labels had landed in phase 3. They
had not. Nothing in the daemon mentioned a label; the claim was wrong and is
corrected here.

What landed now:

| | |
|---|---|
| `label` on `TorrentOptions` | Saved and restored with the torrent, not in a second file keyed by torrent id |
| `label` in the status | So a client can show it and filter on it |
| `label` in `core.get_filter_tree` | A fourth category beside state, tracker and owner |
| `normalise_label` | Lower case, and only the characters the plugin's own validator allowed |

The plugin refused a label with a space in it. This cleans it instead: `My
Films!` becomes `myfilms`. Refusing would mean a torrent silently keeping its
old label because of one character, which is the worse failure.

The Web UI gets labels for free. The sidebar draws whatever categories the
filter tree returns and the filter goes back through `core.get_torrents_status`,
which matches on status keys generically. No front-end change was needed.

Unlike the plugin, a label carries no options of its own. It names a group.

## Watched directories

One key, a list of directories, each with its own location, label and disposal.

The part worth describing is that a file appearing is not the same as a file
being finished. Something is still writing it, and reading it early gets a
truncated torrent whose parse error says nothing about the cause. The plugin
retried a fixed ten times. This waits for the size to stop changing between two
scans, which is the same idea without the arbitrary number.

After a torrent is added the file is renamed out of the way by default,
`a.torrent` becoming `a.torrent.added`. Rename rather than delete, because it
is the only disposal that cannot lose a file the daemon then failed to keep.

## Block list

The only one of the four that hid real work, and it was in two places.

**libtorrent had no IP filter on the bridge.** Nothing crossed it for
`set_ip_filter`, so that is new: a `IpRange { first, last, blocked }` struct,
an ordered list of rules, and a count of the ranges the filter holds for tests
to measure. Two things are checked on the C++ side before libtorrent sees them,
because libtorrent asserts on both rather than reporting them, and an assert is
compiled out of a release build: a range that mixes IPv4 with IPv6, and a range
that ends before it starts.

**The whitelist is not subtracted, it is appended.** libtorrent applies rules
in order and a later one wins where it overlaps, so a whitelist entry has to
come after the blocked range it sits inside. Building it the other way round
produces a filter of exactly the right size that quietly blocks the addresses
it was told not to.

Both text formats Deluge could read are read here, detected from the first line
that says anything:

| Format | A line |
|---|---|
| PeerGuardian, SafePeer, p2p | `Some organisation:1.2.3.4-5.6.7.8` |
| eMule | `001.002.003.004 - 005.006.007.008 , 000 , Some organisation` |

Two details that a naive reader gets wrong. The PeerGuardian range is what
follows the *last* colon, because organisation names contain colons. And every
eMule list writes zero-padded octets, which Rust's own address parser rejects,
so a list would import as zero ranges without a parser that tolerates them.

Lines that will not parse are counted and skipped rather than failing the
import, because a public list of two hundred thousand lines usually has a few.
The count goes in the log next to the number of ranges installed.

Gzip and plain text are unpacked. Zip and bzip2 are detected and refused by
name, which is in the TODO: both would be another dependency, and the lists
people actually use are gzipped.

## Schedule

Twenty-four rows of seven, `[hour][weekday]` with Monday as weekday 0. That is
the shape the plugin stored, so a `button_state` out of an existing
`scheduler.conf` can be pasted into `core.conf` and mean the same thing.
Getting the two indices the wrong way round would silently apply Tuesday's
schedule on Wednesday and still pass a test that filled the whole grid, so
there is a test that fills one cell.

The schedule runs on the hour in local time, which needs the zone database
rather than a fixed offset: a rule written for 9am has to still mean 9am after
a clock change. That is one new dependency, `chrono`, for its clock alone.

It applies only on a change of state, so it does not fight a client that sets a
rate limit by hand within the same hour.

## The plugin interface, removed

| Removed | |
|---|---|
| `Plugin.js` | The plugin base class |
| `preferences/PluginsPage.js` | The preferences page |
| `preferences/InstallPluginWindow.js` | The upload dialog |
| The loader in `UI.js` | `web.get_plugins` on connect, the resource fetch, the enable and disable handlers |
| The registry in `Deluge.js` | `pluginStore`, `createPlugin`, `hasPlugin`, `registerPlugin` |
| Two CSS rules and one icon | `install_plugin.png` |

`web.get_plugins` still answers, because it is in the contract. Nothing shipped
calls it any more.

## Tests

The decisions are pure and tested without a session, a network or a clock: what
the grid says for an hour, whether a file has settled, what a line of a list
means, which rules an import produces. The parts that need libtorrent are
tested against a real session.

| | |
|---|---|
| Gate total | 286, from 207 |
| New in the daemon crate | 70, of which 63 are the features' own |
| New in the libtorrent crate | 8, all on the IP filter |
| New in the web crate | 1, that the plugin interface stays gone |

The IP filter tests measure the filter rather than trusting the call: an empty
filter holds one range per address family, blocking one range splits the space
into four, and an allowed range inside a blocked one splits it into six. That
last one is the whitelist, and it is the test that would have caught building
the rules in the wrong order.

## Verified running

Tests are not the same as the feature working, so both were driven against a
real daemon in the container.

A `.torrent` dropped into a watched directory was picked up on the next scan,
added paused to the configured location with the label `films`, lower-cased
from the `Films` in the configuration, and the file was renamed to
`test.torrent.added`. `torrents.json` carries the label, so it survives a
restart.

A gzipped PeerGuardian list left in `blocklist.cache` was unpacked, read and
installed at startup with no network: three ranges, one unparseable line
skipped, one whitelist entry applied over the top.

## What is not here

In the TODO, with the reasoning: no preferences page for any of the three
settings, `.magnet` files in a watched directory, zip and bzip2 lists, and two
timing edges where a change takes effect on the next scheduled check rather
than at once.
