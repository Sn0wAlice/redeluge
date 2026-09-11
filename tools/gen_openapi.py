#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Write docs/openapi.yaml from the frozen contract.

The HTTP surface is one endpoint, `POST /json`, carrying JSON-RPC. Describing
that in OpenAPI means one path with a request body that is a union over every
method, discriminated by the `method` field, which is exactly what this
generates: one schema per method, its parameters in order, its authorisation
level, and where the Python implementation defined it.

Generated rather than written, for the same reason the contract is extracted
rather than transcribed: a hand-written specification drifts from the
implementation silently, and a drifted specification is worse than none.

    tools/gen_openapi.py            # write docs/openapi.yaml
    tools/gen_openapi.py --check    # fail if it is out of date

No third-party modules: the YAML is emitted directly, because pulling in a
serialiser for one file would add a dependency the gate would have to install.
"""

import argparse
import json
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
CONTRACT = ROOT / 'contract' / 'rpc-api.json'
OUTPUT = ROOT / 'docs' / 'openapi.yaml'

# The daemon reports this to clients, and it is the version of the API rather
# than of this fork: a client checks it to decide what it may call.
API_VERSION = '2.2.1'

AUTH_LEVELS = {
    0: 'none, answered before a session exists',
    1: 'read-only',
    5: 'normal',
    10: 'admin',
}

NAMESPACE_NOTES = {
    'auth': 'Sessions and passwords. Answered by the Web UI server itself.',
    'system': 'Introspection. Answered by the Web UI server itself, and '
    'not in the extracted contract because it carries no decorator.',
    'web': 'The Web UI server: its own configuration, the connection to the '
    'daemon, and the aggregated update the interface polls.',
    'webutils': 'Two methods Deluge exported under a second name. Aliases of '
    'their `web.*` twins.',
    'core': 'The daemon. Forwarded over DelugeRPC, unchanged.',
    'daemon': 'The daemon itself rather than its torrents. Forwarded.',
}

# Answered by the Web UI server rather than forwarded to the daemon. Anything
# else in `web`/`auth`/`system` is local by definition; this is only needed for
# the description, so it stays a plain list.
LOCAL_NAMESPACES = {'auth', 'system', 'web', 'webutils'}


def quote(text):
    """A YAML double-quoted scalar."""
    escaped = text.replace('\\', '\\\\').replace('"', '\\"')
    return f'"{escaped}"'


def block(text, indent):
    """A YAML literal block, for anything with newlines in it."""
    pad = ' ' * indent
    lines = text.strip('\n').rstrip().split('\n')
    body = '\n'.join(f'{pad}  {line}'.rstrip() for line in lines)
    return f'{pad}|-\n{body}'


def schema_name(method):
    """A schema name from a method name: `core.add_torrent_url` -> `CoreAddTorrentUrl`."""
    parts = method.replace('.', '_').split('_')
    return ''.join(part[:1].upper() + part[1:] for part in parts if part)


def describe(method):
    """The prose for one method."""
    lines = []
    if method.get('summary'):
        lines.append(method['summary'].strip())
    level = method.get('auth_level')
    lines.append(f'Authorisation: {AUTH_LEVELS.get(level, level)}.')
    if method['namespace'] in LOCAL_NAMESPACES:
        lines.append('Answered by the Web UI server.')
    else:
        lines.append('Forwarded to the daemon over DelugeRPC.')

    required = [p for p in method['params'] if p['required']]
    optional = [p for p in method['params'] if not p['required']]
    if required:
        lines.append(
            'Required parameters, in order: '
            + ', '.join(f'`{p["name"]}`' for p in required)
            + '.'
        )
    if optional:
        lines.append(
            'Optional, in order after those: '
            + ', '.join(
                f'`{p["name"]}`'
                + (f' (default `{p["default"]}`)' if p['default'] else '')
                for p in optional
            )
            + '.'
        )
    if not method['params']:
        lines.append('Takes no parameters.')
    lines.append(f'Defined in the Python implementation at `{method["source"]}`.')
    return '\n'.join(lines)


def method_schema(method):
    """One request schema: the method name pinned, and its parameters in order."""
    name = schema_name(method['name'])
    out = [
        f'    {name}:',
        '      type: object',
        '      description:',
        block(describe(method), 8),
        '      required: [method]',
        '      properties:',
        '        method:',
        '          type: string',
        f'          const: {quote(method["name"])}',
        '        id:',
        '          $ref: "#/components/schemas/RequestId"',
        '        params:',
    ]

    if not method['params']:
        out += [
            '          type: array',
            '          description: Empty; this method takes no parameters.',
            '          maxItems: 0',
            '          default: []',
        ]
        return '\n'.join(out)

    required = sum(1 for p in method['params'] if p['required'])
    out += [
        '          type: array',
        '          description: Positional, in the order below.',
        f'          minItems: {required}',
        f'          maxItems: {len(method["params"])}',
        '          prefixItems:',
    ]
    for param in method['params']:
        note = param['name']
        if param['annotation']:
            note += f' ({param["annotation"]})'
        if not param['required']:
            note += ' — optional'
            if param['default']:
                note += f', defaults to {param["default"]}'
        out += ['            - description: ' + quote(note)]
    return '\n'.join(out)


# The one method the extractor could not see: `system.listMethods` is answered
# by the JSON handler itself rather than declared with a decorator, so it is
# absent from the contract and present on the wire. Declared here so the
# specification matches what the server actually answers.
EXTRA_METHODS = [
    {
        'name': 'system.listMethods',
        'namespace': 'system',
        'method': 'listMethods',
        'transport': 'web',
        'auth_level': 0,
        'returns': None,
        'params': [],
        'summary': 'Every method this server will answer, its own and the '
        "daemon's, sorted.",
        'source': 'answered by the JSON handler, not declared with @export',
    },
]


def render(contract):
    methods = sorted(contract['methods'] + EXTRA_METHODS, key=lambda m: m['name'])
    namespaces = sorted({m['namespace'] for m in methods})

    header = f'''# Generated by tools/gen_openapi.py from contract/rpc-api.json.
# Do not edit by hand: run the generator instead, and the gate checks it.
openapi: 3.1.0

info:
  title: redeluge JSON-RPC API
  version: {quote(API_VERSION)}
  summary: The HTTP API of the redeluge Web UI server.
  description:
{
        block(
            """
Everything a client does goes through one endpoint, `POST /json`, carrying
JSON-RPC version 1. This is Deluge's own API, unchanged: redeluge reproduces
it method for method so that clients written against the Python server keep
working. The version above is the API's, not the fork's.

A call is `{"method": ..., "params": [...], "id": ...}` and a reply is always
HTTP 200 with `{"result": ..., "error": ..., "id": ...}`. A failure is
reported in `error`, never in the status code, because that is what the
shipped front end expects.

Three methods answer before a session exists: `auth.login`,
`auth.check_session` and `system.listMethods`. Everything else needs the
session cookie that `auth.login` sets.

Methods in the `core.*` and `daemon.*` namespaces are forwarded to the daemon
over DelugeRPC; the rest are answered by the Web UI server itself.
""",
            4,
        )
    }
  license:
    name: GPL-3.0-or-later
    url: https://www.gnu.org/licenses/gpl-3.0.html

servers:
  - url: http://localhost:8112
    description: The Web UI, as the container publishes it.

tags:
'''
    tags = []
    for namespace in namespaces:
        tags.append(
            f'  - name: {namespace}\n    description: '
            + quote(NAMESPACE_NOTES.get(namespace, namespace))
        )
    header += '\n'.join(tags) + '\n'

    paths = (
        """
security:
  - sessionCookie: []

paths:
  /json:
    post:
      summary: Call one method.
      description:
"""
        + block(
            """
The body names the method and its positional parameters. The response is
HTTP 200 whatever happens; read `error` to find out whether the call worked.

`auth.login` is the only method that mints a session: it sets the
`_session_id` cookie, which every later call must carry.
""",
            8,
        )
        + """
      operationId: call
      tags: ["""
        + ', '.join(namespaces)
        + """]
      security:
        - sessionCookie: []
        - {}
      requestBody:
        required: true
        content:
          application/json:
            schema:
              $ref: "#/components/schemas/Request"
      responses:
        "200":
          description: The call was dispatched. `error` says whether it worked.
          headers:
            Set-Cookie:
              description: Sent by `auth.login`, and refreshed on every call.
              schema:
                type: string
          content:
            application/json:
              schema:
                $ref: "#/components/schemas/Response"
        "400":
          description: The body was not a JSON-RPC call.

  /:
    get:
      summary: The Web UI itself.
      description: The rendered page. Everything after it is a call to `/json`.
      operationId: index
      tags: [web]
      security:
        - {}
      responses:
        "200":
          description: The page.
          content:
            text/html:
              schema:
                type: string

  /render/{name}:
    get:
      summary: One rendered template fragment.
      description:
"""
        + block(
            """
The front end fetches a few fragments of markup rather than building them in
JavaScript. Served from the templates compiled into the binary.
""",
            8,
        )
        + """
      operationId: render
      tags: [web]
      security:
        - {}
      parameters:
        - name: name
          in: path
          required: true
          description: The fragment's file name, such as `tab_status.html`.
          schema:
            type: string
      responses:
        "200":
          description: The fragment.
          content:
            text/html:
              schema:
                type: string
        "404":
          description: No such fragment.

  /{asset}:
    get:
      summary: A static asset.
      description:
"""
        + block(
            """
The ExtJS front end, its stylesheets, icons and images, compiled into the
binary rather than read from a directory. A path that matches no asset is
answered with the page, so a reload of a front-end route works.
""",
            8,
        )
        + """
      operationId: asset
      tags: [web]
      security:
        - {}
      parameters:
        - name: asset
          in: path
          required: true
          description: The asset path, such as `js/deluge-all.js`.
          schema:
            type: string
      responses:
        "200":
          description: The asset.

components:
  securitySchemes:
    sessionCookie:
      type: apiKey
      in: cookie
      name: _session_id
      description:
"""
        + block(
            """
Set by `auth.login`. The value is a session id with a four-digit checksum
appended, which is the shape the Python server used; the checksum is not a
security control, the id behind it is.
""",
            8,
        )
        + """

  schemas:
    RequestId:
      description: Echoed back on the response, so a client can pair them up.
      oneOf:
        - type: integer
        - type: string
        - type: "null"

    Error:
      type: object
      description: Present on `error` when a call failed, and `null` otherwise.
      required: [message, code]
      properties:
        message:
          type: string
          description: What went wrong, in words.
        code:
          type: integer
          description:
"""
        + block(
            """
1 for a call that needed a session and did not have one, 2 for a method this
server does not know, 3 for a failure inside the method, 4 for an error the
daemon reported.
""",
            12,
        )
        + """

    Response:
      type: object
      description: Always HTTP 200. Exactly one of `result` and `error` is set.
      required: [result, error, id]
      properties:
        result:
          description: Whatever the method returns. `null` when it failed.
        error:
          oneOf:
            - $ref: "#/components/schemas/Error"
            - type: "null"
        id:
          $ref: "#/components/schemas/RequestId"

    Request:
      description: One call. The `method` field picks which shape applies.
      discriminator:
        propertyName: method
      oneOf:
"""
    )
    refs = '\n'.join(
        f'        - $ref: "#/components/schemas/{schema_name(m["name"])}"'
        for m in methods
    )
    schemas = '\n\n'.join(method_schema(m) for m in methods)

    removed = contract.get('removed', [])
    footer = ''
    if removed:
        names = ', '.join(
            f'`{m["name"]}`' for m in sorted(removed, key=lambda m: m['name'])
        )
        footer = (
            '\n\n# Methods the Python implementation had and redeluge does not.\n'
            '# They managed plugins, and there is no plugin system:\n'
            f'#   {names}\n'
        )

    return header + paths + refs + '\n\n' + schemas + '\n' + footer


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        '--check',
        action='store_true',
        help='fail if the file on disk is not what would be written',
    )
    args = parser.parse_args()

    contract = json.loads(CONTRACT.read_text())
    rendered = render(contract)

    if args.check:
        if not OUTPUT.exists():
            print(f'{OUTPUT} is missing; run tools/gen_openapi.py', file=sys.stderr)
            return 1
        if OUTPUT.read_text() != rendered:
            print(f'{OUTPUT} is out of date; run tools/gen_openapi.py', file=sys.stderr)
            return 1
        print(f'openapi: up to date, {len(contract["methods"])} methods')
        return 0

    OUTPUT.parent.mkdir(parents=True, exist_ok=True)
    OUTPUT.write_text(rendered)
    print(f'wrote {OUTPUT.relative_to(ROOT)}: {len(contract["methods"])} methods')
    return 0


if __name__ == '__main__':
    sys.exit(main())
