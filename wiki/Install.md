# Install

Linux only. There are two supported ways to run redeluge: the container, or
built from source and run under systemd.

If you already run Deluge, read [Migrating from Deluge](Migrating-from-Deluge)
first. The torrent list needs one conversion before the daemon will start.

## The container

The shortest path, and the one that needs nothing installed but Docker.

```bash
git clone https://github.com/retorrent/redeluge
cd redeluge
cp .env.example .env
$EDITOR .env          # set DELUGE_WEB_PASSWORD
docker compose up -d --build
```

The Web UI is then on <http://127.0.0.1:8112>, already connected to the daemon.
The first boot writes `data/config`, wires the Web UI to the local daemon and
applies your password, so there is no trip through the connection manager.

[Docker](Docker) covers the volumes, the environment and what to change before
exposing it.

## From source

You need libtorrent 2.0 and its pkg-config file. On Debian or Ubuntu:

```bash
sudo apt install libtorrent-rasterbar-dev pkg-config build-essential
```

Then the usual:

```bash
cargo build --release
```

The binaries land in `target/release/redeluged` and
`target/release/redeluge-web`. Install them wherever you keep local binaries:

```bash
sudo install -m 755 target/release/redeluged target/release/redeluge-web /usr/local/bin/
```

The Web UI assets are compiled into `redeluge-web`, so there is no asset
directory to copy and nothing to forget to ship.

### Running it by hand

```bash
redeluged            # DelugeRPC on 58846, loopback unless allow_remote is set
redeluge-web         # the Web UI on 8112
```

Both read their configuration from `$DELUGE_CONFIG_DIR`, falling back to
`$XDG_CONFIG_HOME/deluge` and then `~/.config/deluge`. The directory name is
still `deluge` so an existing installation is found where it already is.

On a first run the daemon writes `core.conf` filled with defaults, generates
its own TLS keypair, and creates an auth file. Nothing has to be prepared.

## Under systemd

Two units ship in `packaging/systemd/`. They run as a `redeluge` user, so
create one first:

```bash
sudo useradd --system --create-home --home-dir /var/lib/redeluge redeluge
sudo cp packaging/systemd/redeluged.service packaging/systemd/redeluge-web.service \
        /etc/systemd/system/
sudo systemctl daemon-reload
sudo systemctl enable --now redeluged redeluge-web
```

The web unit is ordered after the daemon, so a reboot brings them up in the
right order. Logs go to the journal:

```bash
journalctl -u redeluged -f
```

## What to do next

Set a Web UI password if you did not do it through the environment, then read
[Configuration](Configuration). Before putting the Web UI anywhere reachable,
read the exposure section of [Docker](Docker): it applies just as much to a
source install.
