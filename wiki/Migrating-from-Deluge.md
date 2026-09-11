# Migrating from Deluge

An existing installation keeps working. The configuration files, the auth file
and the Web UI are unchanged, and any client that spoke to the Python daemon
speaks to this one. One thing has to be converted, and two things change
themselves on first start.

## Convert the torrent list

Deluge stores `torrents.state` as a Python pickle, which names the Python class
it was written from. Nothing but Python can read it, and writing a pickle
reader in Rust would be a bad idea.

```bash
python3 tools/migrate_state.py ~/.config/deluge
```

Or, for the container, against the mounted directory:

```bash
python3 tools/migrate_state.py ./data/config
```

Nothing is deleted. The converter writes `torrents.json` beside the original
and splits `torrents.fastresume` into one file per torrent under
`state/resume/`. Running it twice is harmless; rolling back means not using the
new files.

The daemon refuses to start until this is done, rather than starting with an
empty torrent list. An empty list looks exactly like having lost everything.

### About the converter

It is standard library only and does not import `deluge`. A safe unpickler
substitutes a placeholder for any Deluge class and refuses anything else, so
a tampered state file cannot execute code through it. It is the only Python
that survives the migration, it runs once, and a copy lives in the image at
`/usr/local/share/redeluge/migrate_state.py`.

## What changes by itself, and says so

**Passwords.** An account stored as Deluge's single-round SHA-1 is validated
once and then rewritten as scrypt, with a line in the log. The `localclient`
account is the exception: it stays in plain text because local tools read that
password back out of the file, and hashing it locks the daemon out of itself.

**The daemon certificate.** Deluge wrote an X.509 version 1 certificate, which
modern TLS stacks refuse. If yours is unusable, both files are moved aside with
`.unusable` appended and a new pair is generated. Nothing is overwritten, and
the fingerprint of the new one is printed at startup.

A thin client that pinned the old certificate will need the new fingerprint.
One that verified nothing, which is what the Python client did, notices
nothing.

## What you lose

| Gone | |
|---|---|
| The GTK interface | Use the Web UI, or a thin client |
| The console interface | Same |
| Plugins | Four of them are built in; the rest are gone |
| Translations | The interface is English |
| Windows and macOS | Linux only |

The four that survived are labels, watched directories, the block list and the
schedule. They are features of the daemon now, configured through `core.conf`
rather than through a plugin's own file and RPC namespace. See
[Features](Features) for the shape of each.

Existing settings do not carry across automatically: a `scheduler.conf` from
the plugin is not read, but its `button_state` can be pasted into the
`scheduler` key of `core.conf` and means exactly the same thing. The same goes
for a blocklist URL and a watched directory. The old plugin files are left
alone.

Labels are the exception that needs re-applying: the plugin kept them in its
own file keyed by torrent id, and redeluge keeps a label on the torrent. Set
them again with `core.set_torrent_options`.

## What you gain

Beyond the implementation, five defects of the Python version are fixed rather
than reproduced:

- The daemon starts with current pyOpenSSL, because there is no pyOpenSSL.
- The RPC listener bounds what it buffers and what it decompresses, before
  authentication.
- A failed login returns an error rather than a Python traceback naming paths
  and versions.
- The Web UI password is scrypt, compared in constant time.
- The configuration file is actually written on a first run.

## Rolling back

Keep the Python installation. Nothing this daemon does destroys what it read:
the pickle is left in place, the old certificate is renamed rather than
deleted, and `core.conf` keeps its format. The one asymmetry is that settings
changed under redeluge are written to the same `core.conf` the Python daemon
reads, which is what you want.
