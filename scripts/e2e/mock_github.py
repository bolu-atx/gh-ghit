#!/usr/bin/env python3
"""Serve fixture responses or record requests through the GitHub API."""
import argparse
import hashlib
import json
import os
import re
import sys
import threading
import urllib.error
import urllib.request
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from urllib.parse import parse_qsl, urlencode, urlsplit


def request_key(method, path, body):
    parts = urlsplit(path)
    path = parts.path
    if method != "POST" or path != "/graphql":
        query = urlencode(sorted(parse_qsl(parts.query, keep_blank_values=True)))
        return f"{method} {path}" + (f"?{query}" if query else "")
    query = " ".join(body.get("query", "").split())
    match = re.search(r"\{\s*(?:\w+\s*:\s*)?(\w+)\s*(?:\(|\{)", query)
    root = match.group(1) if match else "unknown"
    variables = json.dumps(body.get("variables", {}), sort_keys=True, separators=(",", ":"))
    digest = hashlib.sha256((query + "\n" + variables).encode()).hexdigest()[:16]
    return f"{method} {path} {root} {digest}"


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--port-file")
    parser.add_argument("--key", action="store_true",
                        help="read a GraphQL request JSON from stdin and print its key")
    parser.add_argument("--fixtures", default="default")
    parser.add_argument("--record", action="store_true")
    args = parser.parse_args()
    if args.key:
        print(request_key("POST", "/graphql", json.load(sys.stdin)))
        return
    if not args.port_file:
        parser.error("--port-file is required unless --key is used")
    fixture = Path(__file__).parent / "fixtures" / (args.fixtures + ".json")
    if not fixture.is_file():
        available = ", ".join(sorted(p.stem for p in fixture.parent.glob("*.json")))
        parser.error(f"unknown fixture set {args.fixtures!r}; available: {available}")
    if args.record and args.fixtures != "default":
        parser.error("record mode requires the default fixture set")
    entries = json.loads(fixture.read_text())
    if args.fixtures != "default":
        defaults = json.loads((fixture.parent / "default.json").read_text())
        entries = {**defaults, **entries}

    lock = threading.Lock()

    class Handler(BaseHTTPRequestHandler):
        def log_message(self, fmt, *values):
            pass

        def handle_patch(self):
            """Record an issue state change and flip the fixture. Never proxied."""
            raw = self.rfile.read(int(self.headers.get("Content-Length", "0")))
            path = urlsplit(self.path).path
            try:
                state = json.loads(raw)["state"]
            except (ValueError, KeyError, TypeError):
                state = None
            entry = entries.get(f"GET {path}")
            if not re.fullmatch(r"/repos/[^/]+/[^/]+/issues/\d+", path) or entry is None or state not in ("open", "closed"):
                status, payload = 422, {"message": "unmatched mock PATCH"}
                print(f"unmatched mock request: PATCH {path} {raw.decode(errors='replace')}", file=sys.stderr, flush=True)
            else:
                with lock:
                    entry["body"]["state"] = state
                status, payload = 200, entry["body"]
                print(f"mock PATCH {path} {json.dumps({'state': state})}", flush=True)
            encoded = json.dumps(payload).encode()
            self.send_response(status)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(encoded)))
            self.end_headers()
            self.wfile.write(encoded)

        def handle_post(self):
            """Record an issue comment and answer 201. Never proxied."""
            path = urlsplit(self.path).path
            if not re.fullmatch(r"/repos/[^/]+/[^/]+/issues/\d+/comments", path):
                return self.handle_request()
            raw = self.rfile.read(int(self.headers.get("Content-Length", "0")))
            try:
                body = json.loads(raw)["body"]
            except (ValueError, KeyError, TypeError):
                body = None
            if body is None:
                status, payload = 422, {"message": "unmatched mock POST"}
                print(f"unmatched mock request: POST {path} {raw.decode(errors='replace')}", file=sys.stderr, flush=True)
            else:
                status, payload = 201, {"id": 1, "body": body}
                print(f"mock POST {path} {json.dumps({'body': body})}", flush=True)
            encoded = json.dumps(payload).encode()
            self.send_response(status)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(encoded)))
            self.end_headers()
            self.wfile.write(encoded)

        def handle_request(self):
            raw = self.rfile.read(int(self.headers.get("Content-Length", "0")))
            try:
                body = json.loads(raw) if raw else {}
                key = request_key(self.command, self.path, body)
            except (ValueError, AttributeError):
                key = f"{self.command} {self.path} invalid JSON"
            if args.record:
                request = urllib.request.Request(
                    "https://api.github.com" + self.path, data=raw or None,
                    headers={"Authorization": "Bearer " + os.environ["E2E_RECORD_TOKEN"],
                             "User-Agent": "ghit-e2e-recorder",
                             "Accept": "application/vnd.github+json",
                             "Content-Type": "application/json"}, method=self.command)
                try:
                    response = urllib.request.urlopen(request, timeout=30)
                except urllib.error.HTTPError as error:
                    response = error
                with response:
                    status = response.status
                    payload = json.loads(response.read())
                with lock:
                    entries[key] = {"status": status, "body": payload}
                    fixture.write_text(json.dumps(entries, indent=2, sort_keys=True) + "\n")
            else:
                entry = entries.get(key)
                if entry is None:
                    print(f"unmatched mock request: {key}", file=sys.stderr, flush=True)
                    entry = {"status": 599, "body": {"message": "unmatched mock request"}}
                status, payload = entry["status"], entry["body"]
            headers = {} if args.record else entry.get("headers", {})
            encoded = json.dumps(payload).encode()
            self.send_response(status)
            for name, value in headers.items():
                self.send_header(name, value)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(encoded)))
            self.end_headers()
            self.wfile.write(encoded)

        do_GET = handle_request
        do_POST = handle_post
        do_PATCH = handle_patch

    server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    port_file = Path(args.port_file)
    temporary = port_file.with_name(port_file.name + ".tmp")
    temporary.write_text(str(server.server_port))
    temporary.replace(port_file)
    server.serve_forever()


if __name__ == "__main__":
    main()
