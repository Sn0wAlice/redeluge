#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Convert a Deluge configuration directory for redeluged.

The Python daemon stored its torrent list as a pickle of
`deluge.core.torrentmanager.TorrentState` objects, and every torrent's resume
data in one bencoded file. Neither is something a Rust daemon should learn to
read, so this converts both, once:

    state/torrents.state      ->  state/torrents.json
    state/torrents.fastresume ->  state/resume/<infohash>.resume

Nothing is deleted. The originals stay where they are, so running this twice is
harmless and rolling back is a matter of not using the new files.

# Why it does not import deluge

A pickle names the class it was written from, and the obvious way to read one is
to have that class importable. That would tie this script to the Python tree,
which is the thing being removed, and leave anyone who upgraded before
converting with a file nothing can read.

So the unpickler is told to return a plain stand-in for any class it meets. The
data is all attributes on a simple object; no code from Deluge ever runs. That
also means this script is safe to point at a file from a machine you do not
control, which the ordinary `pickle.load` would not be.

Usage:
    python3 tools/migrate_state.py <config dir> [--dry-run]
"""

from __future__ import annotations

import argparse
import contextlib
import io
import json
import pickle
import sys
from pathlib import Path
from typing import Any

# Fields redeluged reads, with the value it uses when the pickle has none.
# Anything else in the pickle is carried across untouched rather than dropped:
# an unknown field costs nothing and losing one costs a setting.
DEFAULTS: dict[str, Any] = {
    'torrent_id': '',
    'filename': '',
    'magnet': None,
    'name': None,
    'save_path': None,
    'storage_mode': 'sparse',
    'paused': False,
    'auto_managed': True,
    'max_connections': -1,
    'max_upload_slots': -1,
    'max_upload_speed': -1.0,
    'max_download_speed': -1.0,
    'prioritize_first_last': False,
    'sequential_download': False,
    'file_priorities': [],
    'is_finished': False,
    'stop_ratio': 2.0,
    'stop_at_ratio': False,
    'remove_at_ratio': False,
    'move_completed': False,
    'move_completed_path': None,
    'owner': '',
    'shared': False,
    'super_seeding': False,
    'trackers': [],
}

# In the pickle these are per-tracker libtorrent state; redeluged keeps only
# what the user chose.
TRACKER_FIELDS = ('url', 'tier')


class _FakeTorrentState:
    """Used only by the self-test, and pickled under Deluge's own names.

    Setting the module and qualified name is what makes the pickle say
    `deluge.core.torrentmanager TorrentState`, so the test exercises the class
    substitution rather than a shortcut around it.
    """

    __module__ = 'deluge.core.torrentmanager'
    __qualname__ = 'TorrentState'


class _FakeManagerState:
    __module__ = 'deluge.core.torrentmanager'
    __qualname__ = 'TorrentManagerState'


@contextlib.contextmanager
def _pretend_deluge_is_installed():
    """Puts a stand-in deluge.core.torrentmanager in sys.modules, briefly."""
    import types

    created = []
    for name in ('deluge', 'deluge.core', 'deluge.core.torrentmanager'):
        if name not in sys.modules:
            sys.modules[name] = types.ModuleType(name)
            created.append(name)

    module = sys.modules['deluge.core.torrentmanager']
    module.TorrentState = _FakeTorrentState
    module.TorrentManagerState = _FakeManagerState
    try:
        yield
    finally:
        for name in reversed(created):
            sys.modules.pop(name, None)


class Placeholder:
    """Stands in for any class the pickle names."""

    def __init__(self, *args: Any, **kwargs: Any) -> None:
        self._args = args
        self._kwargs = kwargs

    def __setstate__(self, state: Any) -> None:
        if isinstance(state, dict):
            self.__dict__.update(state)


class SafeUnpickler(pickle.Unpickler):
    """Reads the data without importing, or running, anything from Deluge."""

    def find_class(self, module: str, name: str) -> Any:
        # Refuse anything that is not a Deluge state class. A pickle can name
        # os.system as readily as it can name TorrentState.
        if module.startswith('deluge.') or module == '__builtin__':
            return Placeholder
        if module == 'builtins' and name in ('list', 'dict', 'set', 'tuple'):
            return getattr(__import__('builtins'), name)
        raise pickle.UnpicklingError(
            f'refusing to load {module}.{name}: not a Deluge state class'
        )


def bdecode(data: bytes) -> Any:
    """Just enough bencode to split the fastresume file.

    The blobs inside are opaque and stay bytes; only the outer dictionary is
    read. Written here rather than taken as a dependency because this script
    has to run anywhere, including on a machine that no longer has libtorrent.
    """
    view = memoryview(data)
    position = 0

    def parse() -> Any:
        nonlocal position
        if position >= len(view):
            raise ValueError('truncated bencode')

        marker = view[position]
        if marker == ord('d'):
            position += 1
            out = {}
            while view[position] != ord('e'):
                key = parse()
                out[key] = parse()
            position += 1
            return out
        if marker == ord('l'):
            position += 1
            out_list = []
            while view[position] != ord('e'):
                out_list.append(parse())
            position += 1
            return out_list
        if marker == ord('i'):
            end = data.index(b'e', position)
            value = int(data[position + 1 : end])
            position = end + 1
            return value
        if marker in b'0123456789':
            colon = data.index(b':', position)
            length = int(data[position:colon])
            start = colon + 1
            position = start + length
            return bytes(view[start:position])
        raise ValueError(f'unexpected bencode marker {chr(marker)!r} at {position}')

    return parse()


def read_state(path: Path) -> list[Any]:
    """Reads the pickled torrent list, falling back to the backup."""
    for candidate in (path, path.with_suffix('.state.bak')):
        if not candidate.is_file():
            continue
        try:
            with candidate.open('rb') as handle:
                state = SafeUnpickler(io.BytesIO(handle.read())).load()
        except Exception as ex:  # noqa: BLE001 - any failure means try the backup
            print(f'  could not read {candidate.name}: {ex}', file=sys.stderr)
            continue

        torrents = getattr(state, 'torrents', None)
        if torrents is None:
            print(f'  {candidate.name} holds no torrent list', file=sys.stderr)
            continue
        print(f'  read {len(torrents)} torrents from {candidate.name}')
        return list(torrents)

    return []


def convert_torrent(entry: Any) -> dict[str, Any] | None:
    fields = dict(getattr(entry, '__dict__', {}))
    fields.pop('_args', None)
    fields.pop('_kwargs', None)

    torrent_id = fields.get('torrent_id')
    if not torrent_id:
        return None

    out: dict[str, Any] = {}
    for key, default in DEFAULTS.items():
        out[key] = fields.pop(key, default)

    # Speeds are floats in redeluged; the pickle sometimes has integers.
    for key in ('max_upload_speed', 'max_download_speed', 'stop_ratio'):
        if out[key] is not None:
            out[key] = float(out[key])

    out['trackers'] = [
        {field: tracker.get(field) for field in TRACKER_FIELDS}
        for tracker in (out['trackers'] or [])
        if isinstance(tracker, dict) and tracker.get('url')
    ]

    # An empty name means "use the one from the metadata", which redeluged
    # spells as null rather than an empty string.
    if out['name'] == '':
        out['name'] = None

    out['file_priorities'] = [
        max(0, min(7, int(priority))) for priority in (out['file_priorities'] or [])
    ]

    # Everything the pickle had that this version does not name.
    for key, value in fields.items():
        if key.startswith('_'):
            continue
        out.setdefault(key, value)

    return out


def split_fastresume(path: Path, destination: Path, dry_run: bool) -> int:
    if not path.is_file():
        return 0

    try:
        decoded = bdecode(path.read_bytes())
    except Exception as ex:  # noqa: BLE001
        print(f'  could not read {path.name}: {ex}', file=sys.stderr)
        return 0
    if not isinstance(decoded, dict):
        print(f'  {path.name} is not a dictionary', file=sys.stderr)
        return 0

    written = 0
    for key, blob in decoded.items():
        info_hash = key.decode() if isinstance(key, bytes) else str(key)
        if not isinstance(blob, bytes) or not blob:
            continue
        if not dry_run:
            destination.mkdir(parents=True, exist_ok=True)
            (destination / f'{info_hash}.resume').write_bytes(blob)
        written += 1
    return written


def self_test() -> int:
    """Convert a pickle built here, and check what comes out.

    Runs without the Deluge package and without a real configuration, so it can
    sit in the ordinary test gate. What it proves is the part that is easy to
    get wrong: that a pickle naming a class this script has never seen is read
    at all, and that the fields survive with the right types.
    """
    import tempfile

    failures = []

    def check(name: str, actual: Any, expected: Any) -> None:
        if actual != expected:
            failures.append(f'{name}: {actual!r} != {expected!r}')

    entry = _FakeTorrentState()
    entry.__dict__.update(
        {
            'torrent_id': 'a' * 40,
            'filename': 'example.torrent',
            'magnet': None,
            'name': '',
            'save_path': '/downloads',
            'paused': True,
            'max_connections': 42,
            'max_upload_speed': -1,
            'stop_ratio': 3.5,
            'stop_at_ratio': True,
            'file_priorities': [0, 4, 9],
            'trackers': [
                {'url': 'http://tracker.example/announce', 'tier': 1, 'fails': 7}
            ],
            'a_field_from_a_future_version': 'kept',
        }
    )

    manager = _FakeManagerState()
    manager.torrents = [entry]

    with tempfile.TemporaryDirectory() as scratch:
        root = Path(scratch)
        state_dir = root / 'state'
        state_dir.mkdir()

        # pickle checks the class is importable before it will write the
        # name, so the module is put in place for the length of the dump. The
        # result is a file naming deluge.core.torrentmanager, exactly as the
        # real one does, which is what makes this test exercise the class
        # substitution rather than a shortcut around it.
        with _pretend_deluge_is_installed():
            raw = pickle.dumps(manager, protocol=2)
        assert b'deluge.core.torrentmanager' in raw, 'the pickle should name Deluge'
        (state_dir / 'torrents.state').write_bytes(raw)

        # A bencoded fastresume holding one opaque blob.
        blob = b'd4:testi1ee'
        (state_dir / 'torrents.fastresume').write_bytes(
            b'd40:' + (b'a' * 40) + f'{len(blob)}:'.encode() + blob + b'e'
        )

        if main_with(['--dry-run'], root) != 0:
            failures.append('dry run failed')
        if (state_dir / 'torrents.json').exists():
            failures.append('the dry run wrote a file')

        if main_with([], root) != 0:
            failures.append('conversion failed')

        written = json.loads((state_dir / 'torrents.json').read_text())
        check('version', written['version'], 1)
        check('torrent count', len(written['torrents']), 1)

        torrent = written['torrents'][0]
        check('torrent_id', torrent['torrent_id'], 'a' * 40)
        check('paused', torrent['paused'], True)
        check('max_connections', torrent['max_connections'], 42)
        check('stop_ratio', torrent['stop_ratio'], 3.5)
        # An empty name means "use the metadata", which is null here.
        check('name', torrent['name'], None)
        # Speeds become floats even when the pickle had an integer.
        check('max_upload_speed', torrent['max_upload_speed'], -1.0)
        # Priorities are clamped to what libtorrent accepts.
        check('file_priorities', torrent['file_priorities'], [0, 4, 7])
        # Trackers keep only what the user chose.
        check(
            'trackers',
            torrent['trackers'],
            [{'url': 'http://tracker.example/announce', 'tier': 1}],
        )
        # A field from a later version is carried across rather than dropped.
        check('unknown field', torrent.get('a_field_from_a_future_version'), 'kept')
        # A field the pickle did not have gets its default.
        check('auto_managed', torrent['auto_managed'], True)

        resume = state_dir / 'resume' / f'{"a" * 40}.resume'
        if not resume.is_file():
            failures.append('no resume file was written')
        elif resume.read_bytes() != blob:
            failures.append('the resume blob was altered')

        # The originals are still there.
        if not (state_dir / 'torrents.state').is_file():
            failures.append('the original state file was removed')

        # And running it again is harmless.
        if main_with([], root) != 0:
            failures.append('a second run failed')

    if failures:
        for failure in failures:
            print(f'FAIL {failure}', file=sys.stderr)
        return 1

    print('migrate_state self-test passed')
    return 0


def main_with(extra: list[str], config_dir: Path) -> int:
    """Runs main() with a given directory, for the self-test."""
    saved = sys.argv
    sys.argv = ['migrate_state.py', str(config_dir), *extra]
    try:
        return main()
    finally:
        sys.argv = saved


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.split('\n\n')[0])
    parser.add_argument(
        'config_dir', type=Path, help='the Deluge configuration directory'
    )
    parser.add_argument(
        '--dry-run',
        action='store_true',
        help='report what would be written without writing it',
    )
    args = parser.parse_args()

    state_dir = args.config_dir / 'state'
    if not state_dir.is_dir():
        print(f'{state_dir} does not exist', file=sys.stderr)
        return 1

    target = state_dir / 'torrents.json'
    if target.exists():
        print(f'{target} already exists; nothing to do')
        return 0

    print(f'converting {state_dir}')
    entries = read_state(state_dir / 'torrents.state')

    torrents = []
    for entry in entries:
        converted = convert_torrent(entry)
        if converted is None:
            print('  skipping an entry with no torrent id', file=sys.stderr)
            continue
        torrents.append(converted)

    resume = split_fastresume(
        state_dir / 'torrents.fastresume', state_dir / 'resume', args.dry_run
    )

    payload = {'version': 1, 'torrents': torrents}
    if args.dry_run:
        print(f'  would write {len(torrents)} torrents and {resume} resume files')
        print(json.dumps(payload, indent=2)[:2000])
        return 0

    # Written to a temporary name and renamed, so an interrupted run does not
    # leave a half-written file that looks complete.
    temporary = target.with_suffix('.json.tmp')
    temporary.write_text(json.dumps(payload, indent=2) + '\n', encoding='utf8')
    temporary.replace(target)

    print(f'  wrote {target.name} with {len(torrents)} torrents')
    print(f'  wrote {resume} resume files to state/resume/')
    print('  the original files were left in place')
    return 0


if __name__ == '__main__':
    if '--self-test' in sys.argv:
        sys.exit(self_test())
    sys.exit(main())
