#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Freeze the RPC surface of the Python daemon into a machine-readable contract.

Phase 0 of the Rust migration. The Rust daemon has to answer the same calls with
the same names, arities and authorisation levels, so the contract is extracted
from the source rather than transcribed by hand, and a test re-runs the
extraction to catch drift.

Parsing is done with `ast`, not regexes: the decorators come in two shapes
(`@export` and `@export(LEVEL)`) and signatures carry defaults and annotations.

Usage:
    python tools/extract_contract.py            # write contract/*.json
    python tools/extract_contract.py --check    # fail if the tree has drifted
"""

from __future__ import annotations

import argparse
import ast
import json
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
OUT_DIR = REPO / 'contract'

AUTH_LEVELS = {
    'AUTH_LEVEL_NONE': 0,
    'AUTH_LEVEL_READONLY': 1,
    'AUTH_LEVEL_NORMAL': 5,
    'AUTH_LEVEL_ADMIN': 10,
    'AUTH_LEVEL_DEFAULT': 5,
}
DEFAULT_AUTH_LEVEL = AUTH_LEVELS['AUTH_LEVEL_DEFAULT']

# Where exported methods live, and the RPC namespace each object is registered
# under. The namespace is what a client actually types: `core.add_torrent_url`.
SOURCES = [
    ('deluge/core/core.py', 'Core', 'core', 'daemon'),
    ('deluge/core/daemon.py', 'Daemon', 'daemon', 'daemon'),
    ('deluge/ui/web/json_api.py', 'WebApi', 'web', 'web'),
    ('deluge/ui/web/json_api.py', 'WebUtils', 'webutils', 'web'),
    ('deluge/ui/web/auth.py', 'Auth', 'auth', 'web'),
]

# Dropped in the Rust daemon: the plugin system goes away entirely, so the
# methods that manage it go with it. Recorded here rather than silently skipped
# so the contract states what was removed and why.
REMOVED = {
    'core.get_available_plugins': 'no plugin system',
    'core.get_enabled_plugins': 'no plugin system',
    'core.enable_plugin': 'no plugin system',
    'core.disable_plugin': 'no plugin system',
    'core.upload_plugin': 'no plugin system, and it executed uploaded code',
    'core.rescan_plugins': 'no plugin system',
    'web.get_plugins': 'no plugin system',
    'web.get_plugin_info': 'no plugin system',
    'web.get_plugin_resources': 'no plugin system',
    'web.upload_plugin': 'no plugin system',
}


def auth_level_of(decorator: ast.expr) -> int | None:
    """Return the auth level an @export decorator grants, or None if not one."""
    # Bare `@export`
    if isinstance(decorator, ast.Name) and decorator.id == 'export':
        return DEFAULT_AUTH_LEVEL

    if not isinstance(decorator, ast.Call):
        return None
    func = decorator.func
    if not (isinstance(func, ast.Name) and func.id == 'export'):
        return None

    # `@export(AUTH_LEVEL_X)` or `@export()`
    if not decorator.args:
        return DEFAULT_AUTH_LEVEL
    arg = decorator.args[0]
    if isinstance(arg, ast.Name) and arg.id in AUTH_LEVELS:
        return AUTH_LEVELS[arg.id]
    if isinstance(arg, ast.Constant) and isinstance(arg.value, int):
        return arg.value
    raise ValueError(f'unrecognised auth level expression: {ast.dump(arg)}')


def params_of(node: ast.FunctionDef | ast.AsyncFunctionDef) -> list[dict]:
    """Positional and keyword parameters, minus `self`."""
    args = node.args
    positional = args.posonlyargs + args.args
    if positional and positional[0].arg == 'self':
        positional = positional[1:]

    # Defaults bind to the tail of the positional list.
    pad = len(positional) - len(args.defaults)
    out = []
    for index, arg in enumerate(positional):
        default = None
        has_default = index >= pad
        if has_default:
            default = ast.unparse(args.defaults[index - pad])
        out.append(
            {
                'name': arg.arg,
                'annotation': ast.unparse(arg.annotation) if arg.annotation else None,
                'required': not has_default,
                'default': default,
            }
        )

    for arg, value in zip(args.kwonlyargs, args.kw_defaults):
        out.append(
            {
                'name': arg.arg,
                'annotation': ast.unparse(arg.annotation) if arg.annotation else None,
                'required': value is None,
                'default': ast.unparse(value) if value is not None else None,
            }
        )

    if args.vararg:
        out.append(
            {
                'name': f'*{args.vararg.arg}',
                'annotation': None,
                'required': False,
                'default': None,
            }
        )
    if args.kwarg:
        out.append(
            {
                'name': f'**{args.kwarg.arg}',
                'annotation': None,
                'required': False,
                'default': None,
            }
        )
    return out


def summary_of(node) -> str | None:
    doc = ast.get_docstring(node)
    if not doc:
        return None
    for line in doc.strip().splitlines():
        line = line.strip()
        if line:
            return line
    return None


def extract_methods() -> tuple[list[dict], list[dict]]:
    kept: list[dict] = []
    removed: list[dict] = []

    for rel_path, class_name, namespace, transport in SOURCES:
        tree = ast.parse((REPO / rel_path).read_text(encoding='utf8'))
        target = next(
            (
                n
                for n in ast.walk(tree)
                if isinstance(n, ast.ClassDef) and n.name == class_name
            ),
            None,
        )
        if target is None:
            raise SystemExit(f'class {class_name} not found in {rel_path}')

        for node in target.body:
            if not isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef)):
                continue
            levels = [auth_level_of(d) for d in node.decorator_list]
            levels = [lvl for lvl in levels if lvl is not None]
            if not levels:
                continue

            rpc_name = f'{namespace}.{node.name}'
            entry = {
                'name': rpc_name,
                'namespace': namespace,
                'method': node.name,
                'transport': transport,
                'auth_level': levels[0],
                'returns': ast.unparse(node.returns) if node.returns else None,
                'params': params_of(node),
                'summary': summary_of(node),
                'source': f'{rel_path}:{node.lineno}',
            }
            if rpc_name in REMOVED:
                entry['removed_because'] = REMOVED[rpc_name]
                removed.append(entry)
            else:
                kept.append(entry)

    kept.sort(key=lambda e: e['name'])
    removed.sort(key=lambda e: e['name'])
    return kept, removed


def extract_events() -> list[dict]:
    """Every DelugeEvent subclass the daemon can emit."""
    tree = ast.parse((REPO / 'deluge/event.py').read_text(encoding='utf8'))
    events = []
    for node in tree.body:
        if not isinstance(node, ast.ClassDef):
            continue
        bases = {ast.unparse(b) for b in node.bases}
        if 'DelugeEvent' not in bases:
            continue
        init = next(
            (
                n
                for n in node.body
                if isinstance(n, ast.FunctionDef) and n.name == '__init__'
            ),
            None,
        )
        events.append(
            {
                'name': node.name,
                'args': [p['name'] for p in params_of(init)] if init else [],
                'summary': summary_of(node),
                'source': f'deluge/event.py:{node.lineno}',
            }
        )
    events.sort(key=lambda e: e['name'])
    return events


def _dict_literal(tree: ast.Module, name: str) -> ast.Dict:
    for node in ast.walk(tree):
        if isinstance(node, ast.Assign):
            for target in node.targets:
                if isinstance(target, ast.Name) and target.id == name:
                    if isinstance(node.value, ast.Dict):
                        return node.value
    raise SystemExit(f'{name} is not a dict literal')


def _keys_with_types(dict_node: ast.Dict, source: str) -> list[dict]:
    out = []
    for key, value in zip(dict_node.keys, dict_node.values):
        if not isinstance(key, ast.Constant):
            continue
        if isinstance(value, ast.Constant):
            kind = type(value.value).__name__
            literal = repr(value.value)
        elif isinstance(value, ast.List):
            kind, literal = 'list', ast.unparse(value)
        elif isinstance(value, ast.Dict):
            kind, literal = 'dict', ast.unparse(value)
        elif isinstance(value, ast.UnaryOp):
            kind, literal = 'int', ast.unparse(value)
        else:
            kind, literal = 'computed', ast.unparse(value)
        out.append(
            {'key': key.value, 'type': kind, 'default': literal, 'source': source}
        )
    out.sort(key=lambda e: e['key'])
    return out


def extract_config() -> dict:
    core_tree = ast.parse(
        (REPO / 'deluge/core/preferencesmanager.py').read_text(encoding='utf8')
    )
    web_tree = ast.parse((REPO / 'deluge/ui/web/server.py').read_text(encoding='utf8'))
    return {
        'core': _keys_with_types(
            _dict_literal(core_tree, 'DEFAULT_PREFS'),
            'deluge/core/preferencesmanager.py',
        ),
        'web': _keys_with_types(
            _dict_literal(web_tree, 'CONFIG_DEFAULTS'), 'deluge/ui/web/server.py'
        ),
    }


def extract_alerts() -> list[dict]:
    """libtorrent alert types the daemon registers a handler for.

    These are exactly the alerts the cxx bridge has to flatten. They are
    registered either with a literal name or by iterating a list of names, so
    both forms are resolved. Handlers are written with and without the `_alert`
    suffix; both spellings normalise to one name.
    """
    names: set[str] = set()

    for rel_path in (
        'deluge/core/core.py',
        'deluge/core/torrentmanager.py',
        'deluge/core/alertmanager.py',
        'deluge/core/torrent.py',
    ):
        tree = ast.parse((REPO / rel_path).read_text(encoding='utf8'))

        # Name -> list of string literals, for `for x in alert_handles:` loops.
        list_vars: dict[str, list[str]] = {}
        for node in ast.walk(tree):
            if not isinstance(node, ast.Assign) or not isinstance(node.value, ast.List):
                continue
            literals = [
                el.value
                for el in node.value.elts
                if isinstance(el, ast.Constant) and isinstance(el.value, str)
            ]
            if not literals:
                continue
            for target in node.targets:
                if isinstance(target, ast.Name):
                    list_vars[target.id] = literals

        for node in ast.walk(tree):
            if not isinstance(node, ast.Call) or not node.args:
                continue
            if getattr(node.func, 'attr', None) != 'register_handler':
                continue

            first = node.args[0]
            if isinstance(first, ast.Constant) and isinstance(first.value, str):
                found = [first.value]
            elif isinstance(first, ast.Name) and first.id in list_vars:
                found = list_vars[first.id]
            else:
                # A loop variable: resolve it back to the iterated list.
                found = []
                for enclosing in ast.walk(tree):
                    if not isinstance(enclosing, ast.For):
                        continue
                    if not isinstance(enclosing.target, ast.Name):
                        continue
                    if isinstance(first, ast.Name) and enclosing.target.id != first.id:
                        continue
                    iterated = enclosing.iter
                    if isinstance(iterated, ast.Name) and iterated.id in list_vars:
                        found = list_vars[iterated.id]
                    elif isinstance(iterated, ast.List):
                        found = [
                            el.value
                            for el in iterated.elts
                            if isinstance(el, ast.Constant)
                        ]

            for name in found:
                names.add(name[: -len('_alert')] if name.endswith('_alert') else name)

    if not names:
        raise SystemExit('no alert handlers found, the extractor is out of date')

    return [{'alert': f'{n}_alert', 'handler_key': n} for n in sorted(names)]


def build() -> dict[str, object]:
    kept, removed = extract_methods()
    config = extract_config()
    alerts = extract_alerts()
    events = extract_events()
    return {
        'contract/rpc-api.json': {
            'description': (
                'RPC surface the redeluge daemon must reproduce. Extracted from '
                'the Python tree by tools/extract_contract.py; do not hand-edit.'
            ),
            'auth_levels': AUTH_LEVELS,
            'method_count': len(kept),
            'methods': kept,
            'removed_count': len(removed),
            'removed': removed,
        },
        'contract/events.json': {
            'description': 'Events the daemon broadcasts to subscribed clients.',
            'event_count': len(events),
            'events': events,
        },
        'contract/config-keys.json': {
            'description': 'Configuration keys and their defaults, per config file.',
            'core_count': len(config['core']),
            'web_count': len(config['web']),
            **config,
        },
        'contract/alerts.json': {
            'description': (
                'libtorrent alerts the daemon handles. The cxx bridge has to '
                'flatten each of these into a plain struct.'
            ),
            'alert_count': len(alerts),
            'alerts': alerts,
        },
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        '--check',
        action='store_true',
        help='exit non-zero if the committed contract no longer matches the tree',
    )
    args = parser.parse_args()

    # Once the Python tree is deleted there is nothing left to extract from.
    # The contract in contract/*.json is then final: it was frozen from a tree
    # that existed, and nothing can regenerate it. Saying so beats a gate step
    # that fails for a reason nobody remembers.
    if not (REPO / 'deluge' / 'core' / 'core.py').is_file():
        if args.check:
            print('the Python tree is gone; contract/*.json is final')
            return 0
        print(
            'the Python tree is gone, so there is nothing to extract from.\n'
            'contract/*.json is the frozen contract and cannot be regenerated.',
            file=sys.stderr,
        )
        return 1

    documents = build()
    drifted = []

    for rel_path, payload in documents.items():
        path = REPO / rel_path
        rendered = json.dumps(payload, indent=2, ensure_ascii=False) + '\n'
        if args.check:
            if not path.exists() or path.read_text(encoding='utf8') != rendered:
                drifted.append(rel_path)
        else:
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(rendered, encoding='utf8')
            print(f'wrote {rel_path}')

    if args.check:
        if drifted:
            print('contract is stale for:', ', '.join(drifted), file=sys.stderr)
            print('run: python tools/extract_contract.py', file=sys.stderr)
            return 1
        print('contract matches the source tree')
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
