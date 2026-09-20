#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Runs the front end's checks in a real browser.

The Web UI is twenty thousand lines of ExtJS that nothing tested. It is not
code you can load in a test runner — it wants a document, a layout and a
browser's idea of what is visible — so this serves it to a real browser and
reads the answers back out of the page.

    tools/ui_harness.py            # run the checks, exit non-zero on failure
    tools/ui_harness.py --serve    # just serve it, for looking at by hand

The bundle is built the way `build.rs` builds it, and the ordering is checked
against that rule rather than guessed: a wrong order is a blank page, and it
looks identical to a correct one until it runs.

A headless browser is found on PATH. There is always one on a GitHub runner
and usually one on a workstation; when there is none, this says so and stops
rather than pretending the checks passed.
"""

import argparse
import http.server
import os
import pathlib
import shutil
import socketserver
import subprocess
import sys
import tempfile
import threading

ROOT = pathlib.Path(__file__).resolve().parent.parent
ASSETS = ROOT / 'crates' / 'redeluge-web' / 'assets'
HARNESS = ROOT / 'tools' / 'ui-harness'

# The browsers worth trying, in the order a machine is likely to have them.
BROWSERS = [
    'chromium',
    'chromium-browser',
    'google-chrome',
    'google-chrome-stable',
    '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome',
    '/Applications/Chromium.app/Contents/MacOS/Chromium',
]


def bundle_order(directory: pathlib.Path) -> list[pathlib.Path]:
    """The order `build.rs` concatenates a bundle's sources in.

    Files in a directory that has subdirectories are appended; files in a leaf
    directory are prepended to the whole list. Subdirectories are visited in
    reverse alphabetical order, and a `.order` file moves the names it lists to
    the front of their own directory. The effect is that base classes land
    before the classes that extend them.
    """
    scripts: list[pathlib.Path] = []

    def walk(at: pathlib.Path) -> None:
        directories = sorted(p for p in at.iterdir() if p.is_dir())
        files = sorted(p.name for p in at.iterdir() if p.suffix == '.js')

        order = at / '.order'
        if order.exists():
            for line in order.read_text().splitlines():
                if line.startswith('+ '):
                    wanted = line[2:].strip()
                    if wanted in files:
                        files.remove(wanted)
                        files.insert(0, wanted)

        directories.reverse()
        if not directories:
            for name in reversed(files):
                scripts.insert(0, at / name)
        else:
            scripts.extend(at / name for name in files)
        for subdirectory in directories:
            walk(subdirectory)

    walk(directory)
    return scripts


def concatenate(directory: pathlib.Path) -> bytes:
    out = bytearray()
    for path in bundle_order(directory):
        out += path.read_bytes()
        if not out.endswith(b'\n'):
            out += b'\n'
    return bytes(out)


def build(into: pathlib.Path) -> None:
    """Lays out everything the page loads, next to the page."""
    for name in ('css', 'themes', 'icons', 'js'):
        source = ASSETS / name
        if source.is_dir():
            shutil.copytree(source, into / name, dirs_exist_ok=True)

    (into / 'deluge-all.js').write_bytes(concatenate(ASSETS / 'js' / 'deluge-all'))
    (into / 'ext-extensions.js').write_bytes(
        concatenate(ASSETS / 'js' / 'extjs' / 'ext-extensions')
    )
    for name in ('index.html', 'checks.js'):
        shutil.copy(HARNESS / name, into / name)


class Reporter(http.server.SimpleHTTPRequestHandler):
    """Serves the harness, and takes the report back from it.

    The page posts its results when it has finished rather than being read
    out of a rendered document: a browser asked to dump the DOM decides for
    itself when a page has settled, and one that decides late hangs until a
    timeout. This way the run ends when the checks do.
    """

    results: list[str] = []
    finished = threading.Event()

    def log_message(self, *_args) -> None:  # noqa: D102
        pass

    def do_POST(self) -> None:  # noqa: N802
        length = int(self.headers.get('Content-Length', 0))
        body = self.rfile.read(length).decode('utf-8', 'replace')
        Reporter.results = [line for line in body.splitlines() if line.strip()]
        Reporter.finished.set()
        self.send_response(204)
        self.end_headers()


def serve(directory: pathlib.Path) -> tuple[socketserver.TCPServer, int]:
    handler = lambda *args, **kwargs: Reporter(  # noqa: E731
        *args, directory=str(directory), **kwargs
    )
    server = socketserver.ThreadingTCPServer(('127.0.0.1', 0), handler)
    server.daemon_threads = True
    threading.Thread(target=server.serve_forever, daemon=True).start()
    return server, server.server_address[1]


def find_browser() -> str | None:
    for candidate in BROWSERS:
        found = shutil.which(candidate) or (
            candidate if os.path.exists(candidate) else None
        )
        if found:
            return found
    return None


def run_checks(browser: str, url: str, wait: float = 90.0) -> list[str]:
    """Opens the page and waits for it to say how it went.

    No automation protocol and no driver to install: the page posts its own
    report, and the browser is closed the moment it arrives.
    """
    with tempfile.TemporaryDirectory() as profile:
        browser_process = subprocess.Popen(
            [
                browser,
                '--headless=new',
                '--disable-gpu',
                '--no-sandbox',
                '--no-first-run',
                '--no-default-browser-check',
                '--disable-extensions',
                '--disable-background-networking',
                f'--user-data-dir={profile}',
                url,
            ],
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )
        try:
            Reporter.finished.wait(timeout=wait)
        finally:
            browser_process.terminate()
            try:
                browser_process.wait(timeout=10)
            except subprocess.TimeoutExpired:
                browser_process.kill()

    return Reporter.results


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        '--serve',
        action='store_true',
        help='serve the harness and print its address, for looking at by hand',
    )
    arguments = parser.parse_args()

    with tempfile.TemporaryDirectory() as staging:
        directory = pathlib.Path(staging)
        build(directory)
        server, port = serve(directory)
        url = f'http://127.0.0.1:{port}/index.html'

        if arguments.serve:
            print(f'serving the harness at {url} — ctrl-c to stop')
            try:
                threading.Event().wait()
            except KeyboardInterrupt:
                pass
            return 0

        browser = find_browser()
        if browser is None:
            print('no headless browser on PATH; the front-end checks did not run')
            print(f'tried: {", ".join(BROWSERS)}')
            return 2

        lines = run_checks(browser, url)
        server.shutdown()

    for line in lines:
        print(line)

    failures = [line for line in lines if ': no' in line.lower()]
    if not lines:
        print('the checks did not report back — the page failed to load')
        return 1
    if failures:
        print(f'\n{len(failures)} of {len(lines)} checks failed')
        return 1
    print(f'\n{len(lines)} checks passed')
    return 0


if __name__ == '__main__':
    sys.exit(main())
