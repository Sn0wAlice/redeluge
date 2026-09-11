#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Generate the rencode conformance corpus from the reference implementation.

rencode has no written specification: the Python module is the specification.
So rather than transcribe it and hope, this dumps a corpus of values and their
encodings, and the Rust tests check both directions against it.

Run inside an environment that has the real rencode, which is the redeluge
image:

    docker run --rm -v "$PWD:/src" -w /src redeluge:2.2.1 \\
        python tools/gen_rencode_corpus.py
"""

from __future__ import annotations

import json
import sys
from pathlib import Path

import rencode

OUT = Path(__file__).resolve().parent.parent / 'contract' / 'rencode-corpus.json'


def described(value):
    """Represent a Python value in the typed form the Rust tests parse."""
    if value is None:
        return {'t': 'none'}
    if isinstance(value, bool):
        return {'t': 'bool', 'v': value}
    if isinstance(value, int):
        return {'t': 'int', 'v': str(value)}
    if isinstance(value, float):
        return {'t': 'f32', 'v': repr(value)}
    if isinstance(value, bytes):
        return {'t': 'bytes', 'v': value.hex()}
    if isinstance(value, str):
        return {'t': 'str', 'v': value}
    if isinstance(value, (list, tuple)):
        return {'t': 'list', 'v': [described(item) for item in value]}
    if isinstance(value, dict):
        return {
            't': 'dict',
            'v': [[described(k), described(v)] for k, v in value.items()],
        }
    raise TypeError(f'no description for {type(value)}')


def cases():
    """Every case that pins a boundary in the format, plus real traffic."""
    # Integers: each of these sits on a boundary between two encodings.
    ints = [
        0,
        1,
        43,
        44,
        45,
        127,
        128,
        255,
        256,
        -1,
        -32,
        -33,
        -34,
        -128,
        -129,
        32767,
        32768,
        -32768,
        -32769,
        2147483647,
        2147483648,
        -2147483648,
        -2147483649,
        9223372036854775807,
        -9223372036854775808,
        # Beyond 64 bits, which rencode writes as a decimal string.
        9223372036854775808,
        -9223372036854775809,
        10**40,
        -(10**40),
    ]
    for value in ints:
        yield f'int {value}', value

    # Strings: the fixed-length encoding stops at 64 bytes.
    for length in (0, 1, 63, 64, 65, 300):
        yield f'bytes len {length}', b'x' * length
        yield f'str len {length}', 'y' * length

    yield 'utf8 multibyte', 'héllo wörld ☃ 🦀'
    yield 'utf8 that is 64 bytes once encoded', 'é' * 32
    yield 'bytes with nulls and high bytes', bytes(range(256))

    # Lists and dicts: fixed-length forms stop at 64 and 25 respectively.
    for length in (0, 1, 63, 64, 65):
        yield f'list len {length}', list(range(length))
    for length in (0, 1, 24, 25, 26):
        yield f'dict len {length}', {f'k{i}': i for i in range(length)}

    yield 'none', None
    yield 'true', True
    yield 'false', False

    for value in (0.0, -0.0, 1.5, -1.5, 3.5, 1e10, -1e10):
        yield f'float32 {value}', value

    # Dict keys are not restricted to strings.
    yield 'int keys', {1: 'a', -5: 'b', 1000: 'c'}
    yield 'mixed keys', {1: None, b'raw': True, 'text': False}
    yield 'tuple key', {(1, 2): 'pair'}

    yield (
        'nested',
        {
            'list': [1, [2, [3, [4]]]],
            'dict': {'a': {'b': {'c': None}}},
            'mixed': [{'x': 1}, {'y': [True, False]}],
        },
    )

    # What actually goes over the wire. These are the shapes from
    # deluge/core/rpcserver.py and deluge/ui/client.py.
    yield (
        'rpc request',
        [[0, 'daemon.login', ['localclient', 'secret'], {'client_version': '2.2.1'}]],
    )
    yield 'rpc response', [1, 0, ['2.2.1']]
    yield (
        'rpc error',
        [2, 0, 'BadLoginError', ['Password does not match'], {}, 'Traceback...'],
    )
    yield (
        'rpc event',
        [3, 'TorrentAddedEvent', ['0123456789abcdef0123456789abcdef01234567', False]],
    )
    yield (
        'torrent status',
        {
            b'name': b'debian-12.0.0-amd64-netinst.iso',
            b'progress': 100.0,
            b'state': b'Seeding',
            b'total_size': 658505728,
            b'files': [{b'index': 0, b'path': b'debian.iso', b'size': 658505728}],
            b'peers': [],
            b'is_finished': True,
        },
    )


def main() -> int:
    # The corpus is generated from the reference implementation, which lives in
    # the environment rather than in this repository, so this keeps working
    # after the Python tree is deleted. What it needs is the rencode module.
    entries = []
    for description, value in cases():
        encoded = rencode.dumps(value)

        # Everything must survive its own round trip in the reference
        # implementation, or the corpus is wrong before Rust ever sees it.
        if rencode.loads(encoded) != rencode.loads(
            rencode.dumps(rencode.loads(encoded))
        ):
            raise SystemExit(f'reference implementation is unstable for: {description}')

        # Deluge decodes with decode_utf8=True, and the reference decoder then
        # raises on any string that is not valid UTF-8. A peer can put arbitrary
        # bytes in a torrent name, so this is reachable. Recorded rather than
        # hidden: the Rust decoder falls back to raw bytes instead of failing,
        # and a test pins that divergence.
        try:
            decoded_utf8 = described(rencode.loads(encoded, decode_utf8=True))
        except UnicodeDecodeError:
            decoded_utf8 = None

        entries.append(
            {
                'description': description,
                'value': described(value),
                'encoded': encoded.hex(),
                # What comes back out, which is not always what went in:
                # lists become tuples, and str/bytes depend on decode_utf8.
                'decoded_utf8': decoded_utf8,
                'decoded_raw': described(rencode.loads(encoded)),
            }
        )

    payload = {
        'description': (
            'rencode conformance corpus, generated from the reference Python '
            'implementation by tools/gen_rencode_corpus.py. Do not hand-edit.'
        ),
        'rencode_version': list(rencode.__version__),
        'float_bits': 32,
        'case_count': len(entries),
        'cases': entries,
    }
    OUT.write_text(
        json.dumps(payload, indent=2, ensure_ascii=False) + '\n', encoding='utf8'
    )
    print(f'wrote {OUT.relative_to(OUT.parent.parent)} with {len(entries)} cases')
    return 0


if __name__ == '__main__':
    sys.exit(main())
