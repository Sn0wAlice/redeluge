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
| `core.*` | forwarded to the daemon | 70 |
| `daemon.*` | forwarded to the daemon | 4 |
| `label.*` | forwarded to the daemon | 8 |
| `redeluge.*` | forwarded to the daemon, this fork's own | 2 |

Ask the server itself for the list:

```bash
curl -s -H 'Content-Type: application/json' \
  -d '{"method":"system.listMethods","params":[],"id":1}' \
  http://127.0.0.1:8112/json
```

There is no plugin system, and the Web UI's own plugin-management methods are
gone with it. One plugin's *API* is answered all the same: `core.get_enabled_plugins`
says `["Label"]` and the eight `label.*` methods work, because labels are part
of this daemon and every program built on Deluge's API asks for the plugin
before it will let you set a category. See
[Features](Features) for the details.

`redeluge.*` is the one namespace that is neither Deluge's nor a plugin's. It
holds what this fork added and Deluge has no equivalent for —
`redeluge.get_recent_actions`, the short history of what the daemon did without
being asked, and `redeluge.get_peers`, the running account of what each peer
has done — and it is separate from `core.*` on purpose, so that no client
can mistake it for a Deluge method and no future Deluge method can collide with
it.

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

## Uploading a torrent file

`POST /upload` is the one endpoint that is not a JSON-RPC call. The add dialog
posts a multipart form to it; the answer is the paths the files were staged at,
which then go to `web.get_torrent_info` and `web.add_torrents`.

```bash
curl -b cookies.txt -F 'file=@x.torrent' http://127.0.0.1:8112/upload
```

```json
{"success": true, "files": ["/config/web-uploads/x.torrent"]}
```

It always answers HTTP 200 with a `success` field, because ExtJS's form submit
treats any other status as a transport failure and shows its own message
instead. The content type is `text/html` rather than `application/json`, which
looks wrong and is not: the dialog posts through a hidden iframe, and only
`text/html` makes the browser insert the body unchanged where ExtJS can read it
back. The body is still JSON. It needs the same session cookie as everything else, refuses anything
that does not parse as a torrent, and caps a file at 10 MB. Staged files that
nothing comes back for are swept after an hour.

A path from `/upload` is the only path `web.get_torrent_info` and
`web.add_torrents` will read: without that, an authenticated client could read
any file the server can.

## The two routes that are not JSON

`POST /upload` is above. The other is `GET /flag/<code>`, which the peers tab
asks for once per row: a two-letter country code, answered with a PNG. Anything
that is not two ASCII letters is a 404, because the code comes from the daemon
rather than from the client and a path segment built from data is how a
traversal starts. Countries are empty unless the daemon has a GeoIP database
configured, so on a default install nothing ever asks.

Deluge had a third, `GET /tracker/<host>`, which answered with the tracker's
own favicon: the web server fetched it from the tracker and cached it. redeluge
does not have it. Fetching a favicon means the server making an outbound
request to every host a torrent names, which is not something the interface
should do without being asked, so the tracker column and the tracker filter are
text.

## Known differences from Deluge

- **The plugin-management methods are gone**, ten of them. `web.get_plugin_info`
  answers with nothing rather than erroring, so a client that asks on connect
  does not break. `web.get_plugins` reports the Label plugin, which is the one
  whose API this daemon answers.
- **`label.*` is answered without a plugin behind it.** Labels are part of the
  daemon, so they cannot be turned off; `core.enable_plugin` answers `true` for
  `Label` and `false` for anything else rather than pretending.
- **Tracker icons are gone**, with the `/tracker/<host>` route that served
  them. See above.
- **`web.start_daemon` refuses.** The daemon is a service of its own under
  systemd or the container, so spawning an unsupervised child from the Web UI
  would be wrong. Every other connection-manager call works.
