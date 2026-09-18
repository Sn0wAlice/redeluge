#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Draws the Web UI icons this fork added.

The icons Deluge shipped cover Deluge's interface. The features added here —
the activity history, the peer ledger, the tracker info window, the torrents a
tracker has stopped recognising, the ones nobody has any more — had no icon,
and the choice was between borrowing one that means something else and drawing
them.

They are drawn rather than downloaded so that the set stays one set. The
palette is sampled from the icons already shipped, not chosen: the existing
icons are what decide what this program's red is. Each icon is a 16x16 grid of
characters, which is the size they are drawn at and the size they are shown at,
so what is written here is what appears on the screen.

    python3 tools/draw_icons.py            # write them where the Web UI looks
    python3 tools/draw_icons.py --check    # fail if what is on disk differs
"""

import argparse
import pathlib
import struct
import sys
import zlib

# Sampled from downloading.png, seeding.png, alert.png, queue.png and
# connections.png.
PALETTE = {
    '.': None,  # transparent
    'k': (0x30, 0x30, 0x30),  # outline, from connections.png
    'w': (0xF2, 0xF2, 0xF2),  # paper, face
    'd': (0x55, 0x55, 0x55),  # the set's dark grey
    'b': (0x66, 0x99, 0xFF),  # seeding blue
    'B': (0x1A, 0x6D, 0xB8),  # queue blue, the darker one
    'r': (0xE6, 0x38, 0x1F),  # alert red
    'R': (0xA9, 0x10, 0x0F),  # its shadow
}

# A clock: what happened, and when. Not a list of lines, which at sixteen
# pixels is a grey smudge.
ACTIVITY = [
    '................',
    '.....kkkkkk.....',
    '...kkwwwwwwkk...',
    '..kwwwwdwwwwwk..',
    '..kwwwwwwwwwwk..',
    '.kwwwwBwwwwwwwk.',
    '.kwwwwwBwwwwwwk.',
    '.kdwwwwBBBdwwwk.',
    '.kwwwwwwwwwwwwk.',
    '.kwwwwwwwwwwwwk.',
    '..kwwwwdwwwwwk..',
    '..kkwwwwwwwwkk..',
    '...kkwwwwwwkk...',
    '.....kkkkkk.....',
    '................',
    '................',
]

# Three nodes and the lines between them. Two little people turn to mush at
# this size; a graph does not.
PEERS = [
    '................',
    '..kkkk....kkkk..',
    '.kbbbbk..kbbbbk.',
    '.kbbbbk..kbbbbk.',
    '..kkkk....kkkk..',
    '....dd....dd....',
    '.....dd..dd.....',
    '......d..d......',
    '.......dd.......',
    '.......dd.......',
    '.....kkkkkk.....',
    '....kbbbbbbk....',
    '....kbbbbbbk....',
    '.....kkkkkk.....',
    '................',
    '................',
]

# A torrent the tracker has no record of: the page, and a question mark where
# the answer should be. Deliberately not the alert the error state wears: this
# is the tracker disowning a torrent, not the torrent being broken, and it is
# the one sidebar row that offers to delete what it lists.
UNREGISTERED = [
    '................',
    '.kkkkkkkkk......',
    '.kwwwwwwwkk.....',
    '.kwwwwwwwwkk....',
    '.kwwwwwwwwwk....',
    '.kwwwwwwwwwk....',
    '.kwwwwwwwwwk....',
    '.kwwwwwwwwwk....',
    '.kwwww.RRRRRRR..',
    '.kwww.RrwwwwrrR.',
    '.kwww.RrrrrwrrR.',
    '.kwww.Rrrrwrrrk.',
    '.kkkk.Rrrwrrrrk.',
    '......RrrrrrrrR.',
    '......RrrwrrrrR.',
    '.......RRRRRRR..',
]

# The letter in the disc, for the tracker info window. An `i` rather than a
# magnifying glass: the window reports what the daemon already knew, it does not
# go and look, and a glass promises the second thing.
TRACKER_INFO = [
    '................',
    '.....kkkkkk.....',
    '...kkBBBBBBkk...',
    '..kBBBBBBBBBBk..',
    '.kBBBBBwwBBBBBk.',
    '.kBBBBBwwBBBBBk.',
    '.kBBBBBBBBBBBBk.',
    '.kBBBBwwwwBBBBk.',
    '.kBBBBBwwBBBBBk.',
    '.kBBBBBwwBBBBBk.',
    '..kBBwwwwwwBBk..',
    '..kBBBBBBBBBBk..',
    '...kkBBBBBBkk...',
    '.....kkkkkk.....',
    '................',
    '................',
]

# A swarm with nobody in it. The first attempt drew the peer graph hollow and
# it read as a face at sixteen pixels; a barred circle says "none" and cannot
# be mistaken for anything else.
DEAD = [
    '................',
    '.....kkkkkk.....',
    '...kkwwwwwwkk...',
    '..kwwwwwwwkwwk..',
    '..kwwwwwwkwwwk..',
    '.kwwwwwwkwwwwwk.',
    '.kwwwwwkwwwwwwk.',
    '.kwwwwkwwwwwwwk.',
    '.kwwwkwwwwwwwwk.',
    '.kwwkwwwwwwwwwk.',
    '.kwkwwwwwwwwwwk.',
    '..kkwwwwwwwwwk..',
    '..kwwwwwwwwwwk..',
    '...kkwwwwwwkk...',
    '.....kkkkkk.....',
    '................',
]

# Making a torrent: the page, and a plus where something is being added to it.
# Deluge shipped a page with a pencil here, which is every program's "edit a
# document" and says nothing about hashing a directory. A plus is the one mark
# that reads at sixteen pixels as "there will be a new one of these".
CREATE = [
    '................',
    '.kkkkkkkkk......',
    '.kwwwwwwwkk.....',
    '.kwwwwwwwwkk....',
    '.kwwwwwwwwwk....',
    '.kwddddwwwwk....',
    '.kwwwwwwwwwk....',
    '.kwddddddwwk....',
    '.kwwww..BBBBB...',
    '.kwddd.BBBwBBB..',
    '.kwwww.BBBwBBB..',
    '.kwddd.BwwwwwB..',
    '.kwwww.BBBwBBB..',
    '.kkkkk.BBBwBBB..',
    '........BBBBB...',
    '................',
]

# A folder, for the browser that chooses what to make a torrent from. The set
# had a drive and a page and nothing in between, so a directory in that list
# was either a hard disk or a document. Blue rather than the usual amber: the
# palette is the one the shipped icons set, and it has no amber in it.
FOLDER = [
    '................',
    '................',
    '..kkkkk.........',
    '.kbbbbbkkkkkkk..',
    '.kbbbbbbbbbbbbk.',
    '.kbbbbbbbbbbbbk.',
    '.kBBBBBBBBBBBBk.',
    '.kBBBBBBBBBBBBk.',
    '.kBBBBBBBBBBBBk.',
    '.kBBBBBBBBBBBBk.',
    '.kBBBBBBBBBBBBk.',
    '.kBBBBBBBBBBBBk.',
    '..kkkkkkkkkkkk..',
    '................',
    '................',
    '................',
]

ICONS = {
    'activity': ACTIVITY,
    'create': CREATE,
    'dead': DEAD,
    'folder': FOLDER,
    'peers': PEERS,
    'tracker_info': TRACKER_INFO,
    'unregistered': UNREGISTERED,
}

WEB_ICONS = pathlib.Path('crates/redeluge-web/assets/icons')


def encode(grid):
    """One grid as the bytes of a 16x16 RGBA PNG."""
    raw = bytearray()
    for row in grid:
        raw.append(0)  # no filter: sixteen rows of sixteen pixels
        for character in row:
            colour = PALETTE[character]
            raw += bytes((0, 0, 0, 0)) if colour is None else bytes(colour) + b'\xff'

    def chunk(kind, data):
        crc = zlib.crc32(kind + data) & 0xFFFFFFFF
        return struct.pack('>I', len(data)) + kind + data + struct.pack('>I', crc)

    header = struct.pack('>IIBBBBB', 16, 16, 8, 6, 0, 0, 0)
    return (
        b'\x89PNG\r\n\x1a\n'
        + chunk(b'IHDR', header)
        + chunk(b'IDAT', zlib.compress(bytes(raw), 9))
        + chunk(b'IEND', b'')
    )


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        '--check',
        action='store_true',
        help='fail if the files on disk are not what this draws',
    )
    parser.add_argument('--into', type=pathlib.Path, default=WEB_ICONS)
    arguments = parser.parse_args()

    problems = 0
    for name, grid in ICONS.items():
        if len(grid) != 16 or any(len(row) != 16 for row in grid):
            raise SystemExit(f'{name} is not 16x16')
        drawn = encode(grid)
        path = arguments.into / f'{name}.png'

        if arguments.check:
            on_disk = path.read_bytes() if path.is_file() else b''
            if on_disk != drawn:
                print(f'{path} is not what draw_icons.py draws', file=sys.stderr)
                problems += 1
            continue

        path.write_bytes(drawn)
        print(f'wrote {path}')

    if problems:
        raise SystemExit(f'{problems} icon(s) differ')
    if arguments.check:
        print(f'icons: {len(ICONS)} drawn as committed')


if __name__ == '__main__':
    main()
