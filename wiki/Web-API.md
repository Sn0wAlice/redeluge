# Web API

Everything a client does goes through one endpoint, `POST /json` on port 8112,
carrying JSON-RPC version 1. This is Deluge's own API, unchanged: redeluge
answers it method for method so clients written against the Python server keep
working.

The machine-readable version is on [OpenAPI](OpenAPI). This page is the one
with worked examples.

## The shape of a call

```json
{"method": "core.get_session_status", "params": [[]], "id": 1}
```

And of a reply:

```json
{"result": {}, "error": null, "id": 1}
```

The response is HTTP 200 whatever happens. A failure is reported in `error`,
never in the status code, because that is what the shipped front end expects.

```json
{"result": null, "error": {"message": "Unknown method", "code": 2}, "id": 1}
```

| Code | Meaning |
|---|---|
| 1 | Not authenticated: the call needed a session and did not have one |
| 2 | Unknown method |
| 3 | The method itself failed |
| 4 | The daemon reported an error |

`params` is positional, always a list. A method that takes nothing still wants
`[]`.

## Which methods exist

| Namespace | Answered by | How many |
|---|---|---|
| `auth.*` | the Web UI server | 4 |
| `system.listMethods` | the Web UI server | 1 |
| `web.*` | the Web UI server | 23 |
| `webutils.*` | the Web UI server, aliases of two `web.*` methods | 2 |
| `core.*` | forwarded to the daemon | 66 |
| `daemon.*` | forwarded to the daemon | 4 |

Ask the server itself for the list:

```bash
curl -s -H 'Content-Type: application/json' \
  -d '{"method":"system.listMethods","params":[],"id":1}' \
  http://127.0.0.1:8112/json
```

The plugin-management methods Deluge had are gone, because there is no plugin
system. `web.get_plugins` still answers, truthfully, with nothing.

## Logging in

Three methods answer before a session exists: `auth.login`,
`auth.check_session` and `system.listMethods`. Everything else needs the
`_session_id` cookie that `auth.login` sets.

A `curl` configuration file keeps the rest of this page short. Put this in
`curl.cfg`:

```
request = "POST"
compressed
cookie = "cookies.txt"
cookie-jar = "cookies.txt"
header = "Content-Type: application/json"
header = "Accept: application/json"
url = "http://127.0.0.1:8112/json"
write-out = "\n"
```

Then:

```bash
curl -d '{"method": "auth.login", "params": ["your password"], "id": 1}' -K curl.cfg
```

`true` means the session cookie is now in `cookies.txt`, and every later call
picks it up. Every call also refreshes the expiry, which is the sliding
timeout the Python server had.

## Connecting to the daemon

The Web UI server and the daemon are separate processes, so the server has to
be pointed at one. In the container this is done for you on first boot.

```bash
curl -d '{"method": "web.connected", "params": [], "id": 1}' -K curl.cfg
curl -d '{"method": "web.get_hosts", "params": [], "id": 1}' -K curl.cfg
```

`web.get_hosts` returns `[[hostID, address, port, username], ...]` from
`hostlist.conf`. Connect with the id:

```bash
curl -d '{"method": "web.connect", "params": ["<hostID>"], "id": 1}' -K curl.cfg
```

The result is the full list of methods that daemon offers. `web.disconnect`
does the reverse.

## The calls you will actually use

### Everything the interface shows, in one call

```bash
curl -d '{"method": "web.update_ui", "params": [["name","state","progress","download_payload_rate"], {}], "id": 1}' -K curl.cfg
```

First parameter the status keys you want, second a filter. This is what the
Web UI polls; it returns the torrent list, the filter tree, the session stats
and the connection state together.

### Add a torrent

By magnet, which is a daemon call:

```bash
curl -d '{"method": "core.add_torrent_magnet", "params": ["magnet:?xt=urn:btih:...", {}], "id": 1}' -K curl.cfg
```

By URL:

```bash
curl -d '{"method": "core.add_torrent_url", "params": ["https://example.invalid/x.torrent", {}], "id": 1}' -K curl.cfg
```

By file, base64 encoded:

```bash
curl -d "{\"method\": \"core.add_torrent_file\", \"params\": [\"x.torrent\", \"$(base64 -w0 x.torrent)\", {}], \"id\": 1}" -K curl.cfg
```

The second parameter of each is the options dictionary: `download_location`,
`add_paused`, `label`, `max_download_speed` and the rest.

### Read and change torrents

```bash
# every torrent, a few keys
curl -d '{"method": "core.get_torrents_status", "params": [{}, ["name","state","progress"]], "id": 1}' -K curl.cfg

# one torrent, everything
curl -d '{"method": "core.get_torrent_status", "params": ["<infohash>", []], "id": 1}' -K curl.cfg

# only what is seeding
curl -d '{"method": "core.get_torrents_status", "params": [{"state": "Seeding"}, ["name"]], "id": 1}' -K curl.cfg

# label one
curl -d '{"method": "core.set_torrent_options", "params": [["<infohash>"], {"label": "films"}], "id": 1}' -K curl.cfg

# pause, resume, remove
curl -d '{"method": "core.pause_torrent", "params": [["<infohash>"]], "id": 1}' -K curl.cfg
curl -d '{"method": "core.resume_torrent", "params": [["<infohash>"]], "id": 1}' -K curl.cfg
curl -d '{"method": "core.remove_torrent", "params": ["<infohash>", false], "id": 1}' -K curl.cfg
```

The filter on `core.get_torrents_status` is exact-match on status keys:
`state`, `tracker_host`, `owner`, `label`. `"All"` means no filter on that
field.

### Settings

```bash
curl -d '{"method": "core.get_config", "params": [], "id": 1}' -K curl.cfg
curl -d '{"method": "core.set_config", "params": [{"max_active_limit": 12}], "id": 1}' -K curl.cfg
```

This is also how the four built-in features are configured; see
[Features](Features).

## Events

The daemon emits 22 events. The Web UI server holds a queue per session:
register interest, then poll.

```bash
curl -d '{"method": "web.register_event_listener", "params": ["TorrentFinishedEvent"], "id": 1}' -K curl.cfg
curl -d '{"method": "web.get_events", "params": [], "id": 1}' -K curl.cfg
```

A thin client on the daemon port gets them pushed instead; see
[DelugeRPC](DelugeRPC).

## Talking to the daemon directly

The Web UI server is a convenience, not a requirement. Anything in `core.*` and
`daemon.*` is the daemon's own API, reachable on port 58846 over TLS with the
binary protocol. That is the interface a thin client uses.

## Known differences from Deluge

Two, both documented rather than hidden:

- **There is no `POST /upload`.** Adding a torrent by file works through
  `core.add_torrent_file` as above; the multipart upload the Web UI's own
  add-by-file dialog posts to is not implemented yet.
- **The plugin-management methods are gone**, ten of them. `web.get_plugins`
  and `web.get_plugin_info` answer with nothing rather than erroring, so a
  client that asks on connect does not break.
