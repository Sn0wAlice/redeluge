# redeluge

**A BitTorrent client with a daemon and a Web UI, written in Rust on top of
[libtorrent](https://libtorrent.org). Linux only.**

redeluge is a fork of [Deluge](https://github.com/deluge-torrent/deluge) whose
Python implementation has been replaced by a Rust one. The wire protocol, the
configuration files and the Web UI are unchanged, so existing clients and
existing installations keep working.

Two binaries:

- **`redeluged`** speaks DelugeRPC on port 58846 and drives libtorrent.
- **`redeluge-web`** serves the Web UI on port 8112 and talks to the daemon.

Any client that speaks DelugeRPC works with it, because the protocol is
Deluge's and has not changed.

## Start here

```bash
cp .env.example .env   # set DELUGE_WEB_PASSWORD
docker compose up -d --build
```

The Web UI is then on <http://127.0.0.1:8112>. See [Install](Install) for the
other ways, and [Migrating from Deluge](Migrating-from-Deluge) if you have an
existing installation.

## Using redeluge

| Page | What is on it |
|---|---|
| [Install](Install) | Container, from source, systemd |
| [Docker](Docker) | The image, its volumes and its environment |
| [Configuration](Configuration) | `core.conf`, `web.conf`, and what each key does |
| [Features](Features) | Labels, watched directories, block list, schedule |
| [Migrating from Deluge](Migrating-from-Deluge) | Converting an existing installation |
| [Troubleshooting](Troubleshooting) | What the common failures look like |

## Talking to it

| Page | What is on it |
|---|---|
| [Web API](Web-API) | The JSON-RPC endpoint, with worked `curl` calls |
| [OpenAPI](OpenAPI) | The machine-readable specification, generated from the contract |
| [DelugeRPC](DelugeRPC) | The wire protocol the daemon speaks on 58846 |

## Working on it

| Page | What is on it |
|---|---|
| [Architecture](Architecture) | The crates, the FFI boundary, the contract |
| [Building and Testing](Building-and-Testing) | The build, and the gate that has to stay green |
| [Migration Overview](Migration-Overview) | How Python came out, phase by phase |

## What is not here

The GTK and console interfaces, plugins, and translations. The interface is the
Web UI and it is in English. A thin client still works: it speaks the same RPC.

Four of Deluge's plugins are not missing but built in: labels, watched
directories, the block list and the schedule. There is nothing to install and
no plugin namespace to call. See [Features](Features).

## Where this comes from

The fork point is Deluge `2.2.1.dev0-43`, upstream commit
[`e58075416`](https://github.com/deluge-torrent/deluge/commit/e58075416dedd53636e89b1cd240f86f2e7c2ee0).
Most of what redeluge does was designed by the Deluge authors; the `AUTHORS`
file in the repository credits them and lists the licences of everything the
Web UI serves. redeluge is GPL-3.0-or-later, with Deluge's OpenSSL linking
exception, unchanged.
