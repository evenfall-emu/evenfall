#!/usr/bin/env python3
"""Loopback-only development server. Private fixtures are opt-in, named files."""
import argparse
import functools
import re
from http.server import SimpleHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from urllib.parse import unquote, urlsplit, parse_qs
from threading import Event

parser = argparse.ArgumentParser()
parser.add_argument('--port', type=int, default=8765)
parser.add_argument('--directory', type=Path, default=Path('zig-out/web'))
parser.add_argument('--fixture', action='append', default=[], metavar='NAME=PATH')
parser.add_argument('--validation', action='store_true', help='Enable headless-test completion handshake')
args = parser.parse_args()
fixtures = {}
validations = {}
for entry in args.fixture:
    name, path = entry.split('=', 1)
    if not name or '/' in name or name in ('.', '..'):
        parser.error('Fixture names must be single path segments')
    fixtures[name] = Path(path).resolve(strict=True)


# The player's client-side pages, which all load its index.html.
PAGE = re.compile(r'/(library(/[^/]+)?|player)')


class Handler(SimpleHTTPRequestHandler):
    extensions_map = {**SimpleHTTPRequestHandler.extensions_map, '.wasm': 'application/wasm', '.js': 'text/javascript'}

    def end_headers(self):
        self.send_header('Cache-Control', 'no-store')
        self.send_header('X-Content-Type-Options', 'nosniff')
        super().end_headers()

    def do_GET(self):
        path = unquote(urlsplit(self.path).path)
        if args.validation and path == '/__validation_hold__':
            token = parse_qs(urlsplit(self.path).query).get('token', [''])[0]
            event = validations.setdefault(token, Event())
            event.wait(120)
            self.send_response(200)
            self.send_header('Content-Type', 'image/svg+xml')
            self.end_headers()
            self.wfile.write(b'<svg xmlns="http://www.w3.org/2000/svg" width="1" height="1"/>')
            validations.pop(token, None)
            return
        if path.startswith('/__fixtures__/'):
            fixture = fixtures.get(path.removeprefix('/__fixtures__/'))
            if fixture is None:
                self.send_error(404)
                return
            self.send_response(200)
            self.send_header('Content-Type', 'application/octet-stream')
            self.send_header('Content-Length', str(fixture.stat().st_size))
            self.end_headers()
            with fixture.open('rb') as source:
                import shutil
                shutil.copyfileobj(source, self.wfile)
            return
        if PAGE.fullmatch(path):
            self.path = '/index.html'
        super().do_GET()

    def do_POST(self):
        if not args.validation or urlsplit(self.path).path != '/__validation_result__':
            self.send_error(404)
            return
        token = parse_qs(urlsplit(self.path).query).get('token', [''])[0]
        length = int(self.headers.get('Content-Length', '0'))
        if length > 65536:
            self.send_error(413)
            return
        result = self.rfile.read(length).decode('utf8')
        print(result, flush=True)
        validations.setdefault(token, Event()).set()
        self.send_response(204)
        self.end_headers()


server = ThreadingHTTPServer(('127.0.0.1', args.port), functools.partial(Handler, directory=str(args.directory)))
print(f'ZiGBA: http://127.0.0.1:{args.port} (loopback only)', flush=True)
server.serve_forever()
