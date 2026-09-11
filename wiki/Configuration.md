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
| `move_completed` | `false` | Move on completion (stored, not yet enforced) |
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
| `stop_seed_at_ratio` | `false` | Stored, not yet enforced |
| `stop_seed_ratio` | `2.0` | |

Rate limits are in KiB/s and `-1` means no limit, which is Deluge's convention.
The daemon converts to libtorrent's bytes per second and its `0` for unlimited,
in one place, so you never have to.

### The built-in features

| Key | |
|---|---|
| `autoadd` | Watched directories |
| `blocklist` | The peer block list |
| `scheduler` | The weekly schedule |

Each is one dictionary, and all three are off by default. [Features](Features)
documents what goes in them. They are the only keys redeluge added; a test
fails if any other key appears that is neither in the contract nor declared as
an addition, which is how an invented key was caught once already.

## What is in `web.conf`

| Key | Default | |
|---|---|---|
| `port` | `8112` | |
| `interface` | `0.0.0.0` | |
| `base` | `/` | Path prefix, for a reverse proxy subpath |
| `theme` | `gray` | |
| `session_timeout` | `3600` | Seconds |
| `default_daemon` | unset | The daemon to connect to on start |
| `pwd_sha1`, `pwd_salt` | seeded | The Web UI password, scrypt despite the key name |
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

The fingerprint is printed at startup, which is what you would pin a remote
client against.
