# Docker

The image builds the daemon and the Web UI from this working tree and runs both
in one container. Two Rust binaries and the C++ libtorrent they drive: no
interpreter, no virtualenv, and the Web UI assets are compiled into the web
binary rather than shipped beside it.

## First run

```bash
cp .env.example .env
$EDITOR .env          # set DELUGE_WEB_PASSWORD
docker compose up -d --build
```

The Web UI is then on <http://127.0.0.1:8112>.

The first boot writes `data/config`, wires the Web UI to the local daemon and
applies your password. You land straight on the torrent list, with no trip
through the connection manager.

## What is where

| Path | Contents |
|---|---|
| `Dockerfile` | Two-stage build: Rust binaries, then a runtime with no toolchain |
| `docker/entrypoint.sh` | Root stage: PUID/PGID, ownership, privilege drop |
| `docker/run.sh` | Starts and supervises `redeluged` and `redeluge-web` |
| `docker/Dockerfile.rust` | The build and test environment, not a release image |
| `data/config` | Container `/config`, the configuration directory |
| `data/downloads` | Container `/downloads` |

## Controlling upgrades

Nothing floats. Two inputs decide what ends up in the image.

1. **Application code** is whatever is checked out here. Rebuild to pick it up.
2. **Dependency versions** are pinned in `Cargo.lock`, and the build uses
   `--locked`, so a rebuild cannot quietly resolve something newer.

libtorrent comes from Debian rather than being vendored: trixie ships 2.0.11,
which is the version Deluge is tested against.

```bash
cargo update --dry-run   # what has moved
```

## Publishing an image

`.github/workflows/docker.yml` builds the image and publishes it to this
repository's package registry. It runs only when someone starts it, from the
Actions tab, because publishing follows a version bump rather than a push.

The tag is the version in `[workspace.package]` of `Cargo.toml` and nothing
else; the workflow reads it and refuses a version that does not look like one.
So the release procedure is one edit:

```toml
[workspace.package]
version = "1.0.3"
```

Then run the workflow. It stops rather than replacing a version already in the
registry, so a tag someone has pulled cannot change under them; re-run with
*overwrite* if replacing it is what you mean.

| Input | |
|---|---|
| `push` | Off builds without publishing, which is how to check that a change still compiles |
| `latest` | Whether `latest` moves to this build |
| `checks` | Runs the same gate as `docker/rust.sh` first; roughly doubles the run |
| `overwrite` | Replace a version already published |
| `platforms` | `linux/amd64`, or both architectures through emulation, which is slow |

The published image carries the usual OCI labels, so the version and the commit
it was built from are readable without pulling it:

```bash
docker buildx imagetools inspect ghcr.io/retorrent/redeluge:1.3.1
```

A locally built image is tagged `redeluge:local` and carries no version,
because there is nothing for it to carry: the one number that means anything is
in `Cargo.toml`, which is what the binaries report and what the publish
workflow reads. Since 1.6.0 it is also the version the daemon reports to
clients, where it used to answer Deluge's `2.2.1`.

## Coming from the Python daemon

The torrent list was a Python pickle, which this daemon cannot read. Convert it
once, from the host:

```bash
python3 tools/migrate_state.py ./data/config
```

Nothing is deleted. The container refuses to start until this is done, because
an empty torrent list looks exactly like having lost everything. The script is
standard library only, and is in the image at
`/usr/local/share/redeluge/migrate_state.py` if you need a copy.

## Environment variables

| Variable | Default | Effect |
|---|---|---|
| `DELUGE_WEB_PASSWORD` | unset | Web UI password, applied on first boot |
| `DELUGE_WEB_PASSWORD_RESET` | `0` | Set to `1` to overwrite the stored password once |
| `DELUGE_WEB_PORT` | `8112` | Web UI port inside the container |
| `DELUGE_WEB_INTERFACE` | `0.0.0.0` | Web UI bind address inside the container |
| `DELUGE_WEB_BASE` | `/` | Path prefix, for serving under a reverse proxy subpath |
| `DELUGE_DAEMON_PORT` | `58846` | Daemon RPC port |
| `DELUGE_LOGLEVEL` | `info` | `none`, `critical`, `error`, `warning`, `info` or `debug`, translated to `RUST_LOG` |
| `RUST_LOG` | unset | What the binaries actually read. Wins over `DELUGE_LOGLEVEL`, and can name a module: `redeluge_daemon::features=debug` |
| `PUID` / `PGID` | `1000` | Ownership of `/config` and `/downloads` |
| `UMASK` | `022` | Umask for both processes |

Settings you change in the Web UI persist. The bootstrap only rewrites
`default_daemon`, `interface`, `port` and `base` on each boot, so that container
configuration stays authoritative over how the UI is served.

## Exposure

The compose file publishes the Web UI on loopback only. That is deliberate. The
Web UI authenticates with one shared password, it has no rate limiting on
attempts, and any session it issues is an admin session. The password is
stored as scrypt rather than the single round of SHA-1 the Python server used,
so an online guess is slow; slow is not none. Put a reverse proxy with TLS and
its own authentication in front of it before it is reachable from anywhere
else.

The daemon RPC port is not published and `allow_remote` is `false` in the seeded
`core.conf`. Enable both only if you need a thin client, and remember that the
RPC listener accepts and buffers messages before authenticating.

Peer traffic on 58946 is the one port that genuinely needs to be reachable.

## Behaviour on failure

`docker/run.sh` exits as soon as either process does, so a crashed daemon takes
the container down and `restart: unless-stopped` brings the pair back together.
The healthcheck only probes the Web UI port.

## What is not in the image

Nothing that reads an archive. Block lists are downloaded and unpacked by the
daemon itself, gzip included, so `7zip` and `unrar-free` are not installed.

Peer country flags are not filled in. Deluge looked them up in a
legacy-format `GeoIP.dat`, MaxMind retired that format, and the daemon
reports an empty country rather than pretending otherwise.

No interpreter. `tools/migrate_state.py` is in the image at
`/usr/local/share/redeluge/migrate_state.py` as a file to copy out, not to run
there.

## See also

- [Install](Install) for the source and systemd routes.
- [Configuration](Configuration) for what the seeded `core.conf` contains.
- [Migrating from Deluge](Migrating-from-Deluge) for an existing installation.
