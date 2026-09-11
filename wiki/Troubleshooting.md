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

The Web UI server connects to the daemon once, at startup, and does not
reconnect on its own. If the daemon restarted, restart `redeluge-web` too. In
the container, restarting the container does both.

This is a known gap rather than a design decision.

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

## Cannot log in

The password is whatever `DELUGE_WEB_PASSWORD` was on the first boot, or
Deluge's default of `deluge` if it was never set. To change it in the
container, set `DELUGE_WEB_PASSWORD` and `DELUGE_WEB_PASSWORD_RESET=1` for one
run.

There is no rate limiting on attempts. The password is scrypt, so a guess is
slow, but do not expose the Web UI without something in front of it.

## A torrent will not add

**From a watched directory.** The scan only reads `.torrent`, only at the top
level of the directory, and only once the file has been the same size on two
consecutive scans. A file still being written is deliberately ignored. Turn the
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
| `zip lists are not supported` | Only gzip and plain text are read |
| `could not download the block list` | With the URL and the reason; it retries `try_times` |
| a high `skipped` count | The file is probably the other format, or not a list |

Nothing happens at all if `enabled` is false, or if the cached copy is not yet
stale and there is no cache. Set `check_after_days` to 1 to force a refetch
within the day.

## The schedule did nothing

It is evaluated on the hour, so turning it on takes effect at the next hour
boundary. Look for `the schedule changed` in the log, which names the state.

It is in local time. In a container that means UTC unless you set `TZ`.

The grid is `[hour][weekday]` with Monday as weekday 0. Indexed the other way
round it applies Tuesday's rules on Wednesday, silently.

## Something is set and does nothing

A short list of options that are stored and reported faithfully but not yet
acted on:

- Move on completion
- Stop and remove at ratio
- Peer country, which is always empty
- Progress events while a torrent is being created

These are in the repository's `TODO.md` with the reasoning. They are gaps, not
mysteries.

## Reporting a problem

Include the version line from the daemon's first log line, which carries both
the reported version and the libtorrent it linked against, and say whether you
came from a Python Deluge installation.
