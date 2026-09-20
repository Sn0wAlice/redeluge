# Configuration

Three files, in the configuration directory, in Deluge's own format. An
existing installation is read as it is; nothing has to be converted except the
torrent list, which [Migrating from Deluge](Migrating-from-Deluge) covers.

| File | Read by | Holds |
|---|---|---|
| `core.conf` | `redeluged` | Everything about torrents, the session and the four built-in features |
| `web.conf` | `redeluge-web` | The Web UI's own port, theme and session timeout |
| `hostlist.conf` | `redeluge-web` | Which daemons the Web UI may connect to |
| `auth` | both | Accounts, one per line |

The directory is `$DELUGE_CONFIG_DIR`, then `$XDG_CONFIG_HOME/deluge`, then
`~/.config/deluge`. In the container it is `/config`.

## The file format

Two concatenated JSON objects: a version header, then the settings.

```json
{
    "file": 1,
    "format": 1
}{
    "daemon_port": 58846,
    "max_active_limit": 8
}
```

That is not a mistake, it is Deluge's format, and redeluge writes it back the
same way with sorted keys and four-space indent, so a diff between the two
implementations stays readable.

The daemon writes the file on first start with every default filled in, which
the Python daemon did not do. If a key is missing it is added on load and the
one you set is kept.

## Changing settings

Do not edit `core.conf` while the daemon is running: it holds the settings in
memory and writes them back on a timer and at shutdown, so your edit is lost.
Use the API instead, which is what every client does:

```bash
core.set_config({"max_active_limit": 12})
```

Two rules the daemon enforces, both deliberate. A key that does not exist is
refused, so a typo in a client cannot add a setting nothing reads. And a value
of the wrong type is refused, except that integers and floats are
interchangeable, because a client that sends `200` where `200.0` is stored must
not break the setting.

## What is in `core.conf`

The 77 keys Deluge had, unchanged, plus three redeluge adds. The full list with
types and defaults is in `contract/config-keys.json` in the repository, which is
extracted from the Python source rather than written by hand. The ones worth
knowing:

### Where things go

| Key | Default | |
|---|---|---|
| `download_location` | `~/Downloads` | Where torrents are saved |
| `move_completed` | `false` | Move the files when the torrent finishes |
| `move_completed_path` | `~/Downloads` | Where to |
| `torrentfiles_location` | `<config>/torrents` | Copies of added `.torrent` files |
| `copy_torrent_file` | `false` | Whether to keep those copies |

### Network

| Key | Default | |
|---|---|---|
| `listen_ports` | `[6881, 6891]` | Peer port range |
| `random_port` | `true` | Pick one at random inside it |
| `listen_interface` | `""` | Bind address, empty for all |
| `outgoing_interface` | `""` | Source interface for outgoing connections |
| `dht`, `upnp`, `natpmp`, `lsd`, `utpex` | `true` | Peer discovery |
| `daemon_port` | `58846` | The RPC listener |
| `allow_remote` | `false` | Bind the RPC listener beyond loopback |

### Limits

| Key | Default | |
|---|---|---|
| `max_download_speed`, `max_upload_speed` | `-1.0` | KiB/s, -1 for no limit |
| `max_connections_global` | `200` | |
| `max_upload_slots_global` | `4` | |
| `max_active_limit` | `8` | Queue: how many torrents run at once |
| `max_active_downloading` | `3` | |
| `max_active_seeding` | `5` | |
| `stop_seed_at_ratio` | `false` | Pause a torrent at its share ratio |
| `stop_seed_ratio` | `2.0` | |

Rate limits are in KiB/s and `-1` means no limit, which is Deluge's convention.
The daemon converts to libtorrent's bytes per second and its `0` for unlimited,
in one place, so you never have to.

### The built-in features

| Key | |
|---|---|
| `autoadd` | Watched directories |
| `blocklist` | The peer block list |
| `countrydb` | The country database that gives peers their flag |
| `disk_space` | Pausing downloads before the disk runs out |
| `idle_pause` | Pausing downloads that get nowhere, so the queue can move |
| `label` | The labels that exist, and what each applies |
| `peers` | Whether to keep a history of what each peer has done, and for how long |
| `scheduler` | The weekly schedule |
| `tracker` | Per-tracker rules: label, move or remove the torrents of one tracker |
| `webhook` | Where to post when a torrent finishes, arrives or breaks, or when a tracker stops answering |

Each has a preferences page in the Web UI as well, except `tracker`, which is
reached by right-clicking a tracker in the sidebar and choosing *Settings*, and
`peers`, which is set in the Peers window itself.

Each is one dictionary. All are off by default except `disk_space`, which is on
because it only ever declines to write to a disk with no room on it and undoes
itself as soon as there is room; `label` and `tracker` have nothing to turn off
and simply start empty. [Features](Features)
documents what goes in them. They are the only keys redeluge added; a test
fails if any other key appears that is neither in the contract nor declared as
an addition, which is how an invented key was caught once already.

## Settings with no control in the interface

Every key below is read and written by the API, because a client that speaks
Deluge's protocol expects all of them. What some of them no longer have is a
control in the preferences window, and the reason is always the same: nothing
acts on the value.

| Key | Why there is no control |
|---|---|
| `enc_in_policy`, `enc_out_policy`, `enc_level` | Never passed to libtorrent |
| `cache_size`, `cache_expiry` | libtorrent 2.0 has no disk cache; it maps files into memory instead |
| `utpex` | libtorrent 2.0 has no setting for peer exchange |
| `outgoing_ports`, `random_outgoing_ports` | No setting for the source port of an outgoing connection |
| `force_proxy` | Dropped by libtorrent 2.0; the three "proxy this kind of connection" switches are what it meant |
| `new_release_check`, `send_info`, `info_sent` | There is no update service to ask and nothing is sent anywhere |
| `port`, `interface`, `https`, `cert`, `pkey` | How the server is reached is the container's business; the server refuses to change its own listener from a browser |
| `enabled_plugins`, `plugins_location` | There is no plugin system |
| `path_chooser_*`, `download_location_paths_list`, `move_completed_paths_list` | Conveniences of the GTK client, which is not here |

The keys that are defaults for the next torrent, on the other hand, do now
reach one: `add_paused`, `download_location`, `move_completed`,
`move_completed_path`, `pre_allocate_storage`, `prioritize_first_last_pieces`,
`sequential_download`, `stop_seed_at_ratio`, `stop_seed_ratio`,
`remove_seed_at_ratio`, `queue_new_to_top`, `copy_torrent_file`,
`torrentfiles_location` and the four `*_per_torrent` limits are read when a
torrent is added, whether by a client, by URL or from a watched directory. A
dictionary the client sends wins over them, which is Deluge's order.

## What is in `web.conf`

| Key | Default | |
|---|---|---|
| `port` | `8112` | |
| `interface` | `0.0.0.0` | |
| `base` | `/` | Path prefix, for a reverse proxy subpath |
| `theme` | `dark` | `dark` or `white`. The names `gray`, `blue` and `access` are the old ones and are still accepted; they are stored as the new one |
| `language` | `""` | Always empty: there is one language |
| `session_timeout` | `3600` | Seconds |
| `poll_interval` | `2000` | How often the interface polls, in milliseconds; Preferences, Interface |
| `delta_updates` | `true` | Answer a poll with the difference since the last one rather than the whole torrent list; Preferences, Interface |
| `default_daemon` | unset | The daemon to connect to on start |
| `pwd_sha1`, `pwd_salt` | seeded | The Web UI password, scrypt despite the key name |
| `daemon_fingerprints` | absent | Host id to sha256, to pin a remote daemon's certificate; Connection Manager, Edit |
| `https`, `cert`, `pkey` | `false` | TLS on the Web UI itself |
| `show_sidebar`, `sidebar_show_zero` | | What the interface remembers |

The 19 web keys are also in `contract/config-keys.json`.

## Accounts

The `auth` file is one account per line, `username:password:level`. Levels are
0 none, 1 read-only, 5 normal, 10 admin. A line with no level still parses,
which is what an old file looks like; `localclient` is an administrator and
anything else gets the normal level.

Passwords are stored as scrypt. A password written by the Python daemon as a
plain string still works and is rewritten as scrypt the first time it is used,
with a line in the log saying so.

The `localclient` account is the exception and stays in plain text. It exists
so that tools on the same machine can read the password back out of the file;
hashing it locks the daemon out of itself. It is never promoted.

The daemon re-reads the file when it changes, so an account can be added
without a restart.

## TLS

The daemon generates its own keypair on first start, in `ssl/` under the
configuration directory. An existing Deluge keypair is used if it can be: a
certificate that is X.509 version 1, which is what Deluge wrote and what
modern TLS stacks refuse, is moved aside with `.unusable` appended and a new
pair is generated. Nothing is overwritten.

The fingerprint is printed at startup. Paste it into *Pin sha256* in the
Connection Manager's Edit window, or put it in `web.conf` directly:

```json
{"daemon_fingerprints": {"<host id>": "c91c7c0e…"}}
```

The host id comes from `web.get_hosts`. Clearing the field removes the pin. Without a pin the client encrypts and
verifies nothing, which is what the Python client did; connecting to a
non-loopback daemon without one logs a warning rather than refusing, because
refusing would break every existing remote setup.
