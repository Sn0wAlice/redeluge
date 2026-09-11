> **Fork notice:** redeluge began as a fork of [Deluge](https://github.com/deluge-torrent/deluge),
> based on `deluge-2.2.1.dev0-43` (upstream commit [`e58075416`](https://github.com/deluge-torrent/deluge/commit/e58075416dedd53636e89b1cd240f86f2e7c2ee0)).
> The Python implementation has been replaced by a Rust one. The wire protocol,
> the configuration files and the Web UI are unchanged, so existing clients and
> existing installations keep working.

# redeluge

![](./.github/banner.png)

A BitTorrent client with a daemon and a Web UI, written in Rust on top of
[libtorrent](https://libtorrent.org). Linux only.

- **`redeluged`** speaks DelugeRPC on port 58846 and drives libtorrent.
- **`redeluge-web`** serves the Web UI on port 8112 and talks to the daemon.

Any client that speaks DelugeRPC works with it, because the protocol is
Deluge's and has not changed.

## Run it

```bash
cp .env.example .env   # set DELUGE_WEB_PASSWORD
docker compose up -d --build
```

The Web UI is then on <http://127.0.0.1:8112>, already connected to the daemon.
See [Docker](https://github.com/Sn0wAlice/redeluge/wiki/Docker) in the wiki.

## Build it

Needs libtorrent 2.0 and its pkg-config file. On Debian or Ubuntu:

```bash
sudo apt install libtorrent-rasterbar-dev pkg-config build-essential
cargo build --release
```

The binaries land in `target/release/`. `docker/rust.sh` runs the whole test
gate in a container, which is how it runs in development. See
[Building and Testing](https://github.com/Sn0wAlice/redeluge/wiki/Building-and-Testing).

## Coming from the Python Deluge

Convert the torrent list once. Nothing is deleted, and the daemon refuses to
start until it is done rather than presenting an empty list:

```bash
python3 tools/migrate_state.py ~/.config/deluge
```

`core.conf`, `web.conf`, `hostlist.conf` and the auth file are read as they
are. Two things change on first start, both announced in the log: a password
stored as the old single-round SHA-1 is rewritten as scrypt, and the daemon
certificate is regenerated because the one Deluge wrote is X.509 version 1,
which rustls will not use. The old files are kept. The full account is in
[Migrating from Deluge](https://github.com/Sn0wAlice/redeluge/wiki/Migrating-from-Deluge).

## What is not here

The GTK and console interfaces, plugins, and translations. The interface is the
Web UI and it is in English. A thin client still works: it speaks the same RPC.

Four of the plugins are not missing, they are built in: labels, watched
directories, the block list and the schedule. There is nothing to install and
no plugin namespace to call; each is a key of `core.conf`. See
[Features](https://github.com/Sn0wAlice/redeluge/wiki/Features).

## Documentation

**The documentation is the [wiki](https://github.com/Sn0wAlice/redeluge/wiki).**
It is generated from `wiki/` in this repository, so send a pull request against
those files rather than editing pages in the wiki interface.

| | |
|---|---|
| [Install](https://github.com/Sn0wAlice/redeluge/wiki/Install) | Container, from source, systemd |
| [Configuration](https://github.com/Sn0wAlice/redeluge/wiki/Configuration) | `core.conf`, `web.conf`, accounts, TLS |
| [Features](https://github.com/Sn0wAlice/redeluge/wiki/Features) | Labels, watched directories, block list, schedule |
| [Web API](https://github.com/Sn0wAlice/redeluge/wiki/Web-API) | The JSON-RPC endpoint, with worked curl calls |
| [OpenAPI](https://github.com/Sn0wAlice/redeluge/wiki/OpenAPI) | The specification, generated from the contract |
| [Architecture](https://github.com/Sn0wAlice/redeluge/wiki/Architecture) | The crates, the FFI boundary, the contract |
| [Migration](https://github.com/Sn0wAlice/redeluge/wiki/Migration-Overview) | How Python came out, phase by phase |

In this repository: [`docs/openapi.yaml`](docs/openapi.yaml) is the API
specification, [CHANGELOG.md](CHANGELOG.md) records where Deluge became
redeluge, and [TODO.md](TODO.md) is what is outstanding.

## Contact

- [Homepage](https://deluge-torrent.org) and [forum](https://forum.deluge-torrent.org) for upstream Deluge
- Upstream [user guide](https://dev.deluge-torrent.org/wiki/UserGuide), most of which still applies

## Licence

GPL-3.0-or-later, with Deluge's OpenSSL linking exception, unchanged from
upstream. See [LICENSE](LICENSE). [AUTHORS](AUTHORS) credits the Deluge
authors and lists the licences of the bundled front end.
[CHANGELOG.md](CHANGELOG.md) records where the fork happened and what changed.
