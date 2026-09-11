#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Generate DelugeRPC frames with the Python implementation, for Rust to check.

Uses deluge's own transfer code path so the frames are exactly what the daemon
puts on the wire, not a reimplementation of it.

    docker run --rm -v "$PWD:/src" -w /src redeluge:2.2.1 \\
        python tools/gen_rpc_frames.py
"""

from __future__ import annotations

import json
import struct
import sys
import zlib
from pathlib import Path

import rencode

OUT = Path(__file__).resolve().parent.parent / 'contract' / 'rpc-frames.json'

PROTOCOL_VERSION = 1
MESSAGE_HEADER_FORMAT = '!BI'


def frame(data) -> bytes:
    """The exact body of deluge.transfer.DelugeTransferProtocol.transfer_message."""
    body = zlib.compress(rencode.dumps(data))
    return struct.pack(
        f'{MESSAGE_HEADER_FORMAT}{len(body)}s', PROTOCOL_VERSION, len(body), body
    )


def cases():
    yield (
        'login request',
        [[0, 'daemon.login', ['localclient', 'abc123'], {'client_version': '2.2.1'}]],
    )
    yield 'info request', [[1, 'daemon.info', [], {}]]
    yield (
        'batched requests',
        [
            [2, 'core.get_session_status', [[]], {}],
            [3, 'core.get_free_space', [None], {}],
        ],
    )
    yield 'response', [1, 0, ['2.2.1']]
    yield 'response with a dict', [1, 4, [{'download_rate': 0.0, 'upload_rate': 0.0}]]
    yield (
        'error with traceback',
        [
            2,
            0,
            'BadLoginError',
            ['Password does not match'],
            {},
            'Traceback (most recent call last):\n  File "/opt/venv/.../rpcserver.py", line 1\n',
        ],
    )
    yield (
        'event',
        [3, 'TorrentAddedEvent', ['0123456789abcdef0123456789abcdef01234567', False]],
    )
    yield 'event without args', [3, 'SessionResumedEvent', []]
    yield 'empty list', []
    yield (
        'large status response',
        [
            1,
            9,
            [
                {
                    f'torrent{i}': {
                        'name': f'name {i}',
                        'progress': float(i),
                        'state': 'Seeding',
                    }
                    for i in range(200)
                }
            ],
        ],
    )


def main() -> int:
    entries = []
    for description, data in cases():
        encoded = frame(data)
        entries.append(
            {
                'description': description,
                'frame': encoded.hex(),
                'body_size': len(encoded) - 5,
                'decoded': repr(
                    rencode.loads(zlib.decompress(encoded[5:]), decode_utf8=True)
                ),
            }
        )

    # A compression bomb: small on the wire, enormous once inflated. The Python
    # daemon inflates this without a bound, before authentication.
    bomb_payload = zlib.compress(b'\x00' * (200 * 1024 * 1024))
    bomb = struct.pack(
        f'{MESSAGE_HEADER_FORMAT}{len(bomb_payload)}s',
        PROTOCOL_VERSION,
        len(bomb_payload),
        bomb_payload,
    )

    payload = {
        'description': (
            'DelugeRPC frames produced by the reference implementation, by '
            'tools/gen_rpc_frames.py. Do not hand-edit.'
        ),
        'protocol_version': PROTOCOL_VERSION,
        'frame_count': len(entries),
        'frames': entries,
        'compression_bomb': {
            'description': (
                f'{len(bomb)} bytes on the wire that inflate to '
                f'{200 * 1024 * 1024} bytes. Reachable before authentication.'
            ),
            'frame': bomb.hex(),
            'inflated_size': 200 * 1024 * 1024,
        },
    }
    OUT.write_text(
        json.dumps(payload, indent=2, ensure_ascii=False) + '\n', encoding='utf8'
    )
    print(
        f'wrote {OUT.name}: {len(entries)} frames, bomb is {len(bomb)} bytes on the wire'
    )
    return 0


if __name__ == '__main__':
    sys.exit(main())
