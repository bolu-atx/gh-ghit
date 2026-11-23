#!/usr/bin/env python3
"""Serve the Jira Cloud fixture that jtui's e2e scenarios run against.

Endpoints: `GET /rest/api/3/myself`, `GET /rest/api/3/project/<KEY>`,
`POST /rest/api/3/search/jql` with `nextPageToken` paging,
`GET /rest/api/3/issue/<KEY>-<n>` and its `/comment` list. Every request
needs the fixture's Basic auth pair; anything else answers 401.

Fixture field values that look like `@now-3h` are resolved to Jira
timestamps at request time, so windowed lists never age out. Detail and
comment data lives under `issue_details`, where #315 and #316 extend it.

Requests are logged to stdout, which the runner keeps per scenario.
"""
import argparse
import base64
import json
import re
import sys
import threading
from datetime import UTC, datetime, timedelta
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from urllib.parse import urlsplit

FIXTURE = Path(__file__).parent / "fixtures" / "jira-default.json"

PROJECT_PATH = re.compile(r"^/rest/api/3/project/([A-Za-z][A-Za-z0-9_]*)$")
ISSUE_PATH = re.compile(r"^/rest/api/3/issue/([A-Za-z][A-Za-z0-9_]*-[0-9]+)(/comment)?$")
SEARCH_PAGE = re.compile(r"^page-(\d+)$")
PROJECT_JQL = re.compile(r'project\s*=\s*"?([A-Za-z][A-Za-z0-9_]*)"?')
NOW_FIELD = re.compile(r"^@now(?:([+-])(\d+)([mhd]))?$")


def jira_stamp(at):
    return at.astimezone(UTC).strftime("%Y-%m-%dT%H:%M:%S.000+0000")


def resolve_now(value, now):
    """Replace `@now`, `@now-3h` and `@now-20d` strings in `value`."""
    if isinstance(value, dict):
        return {key: resolve_now(item, now) for key, item in value.items()}
    if isinstance(value, list):
        return [resolve_now(item, now) for item in value]
    if not isinstance(value, str):
        return value
    match = NOW_FIELD.match(value)
    if not match:
        return value
    sign, amount, unit = match.groups()
    if not sign:
        return jira_stamp(now)
    steps = {"m": timedelta(minutes=1), "h": timedelta(hours=1), "d": timedelta(days=1)}
    delta = steps[unit] * int(amount)
    return jira_stamp(now + delta if sign == "+" else now - delta)


def authorized(header, auth):
    if not header or not header.startswith("Basic "):
        return False
    try:
        pair = base64.b64decode(header[6:]).decode()
    except (ValueError, UnicodeDecodeError):
        return False
    return pair == f"{auth['email']}:{auth['token']}"


def issue_entry(entries, key, fields):
    detail = entries["issue_details"][key]
    wanted = detail["fields"]
    if fields:
        wanted = {name: value for name, value in wanted.items() if name in fields}
    return {"id": detail.get("id", key), "key": key, "fields": wanted}


def search(entries, body):
    if not isinstance(body, dict):
        return 400, {"errorMessages": ["unreadable search body"]}
    match = PROJECT_JQL.search(str(body.get("jql", "")))
    if not match or match.group(1) not in entries["projects"]:
        return 400, {"errorMessages": ["unknown project in the JQL"]}
    pages = entries["search"]["pages"]
    index = 0
    token = body.get("nextPageToken")
    if token is not None:
        found = SEARCH_PAGE.fullmatch(str(token))
        if not found or not 1 <= int(found.group(1)) - 1 < len(pages):
            return 400, {"errorMessages": ["unknown nextPageToken"]}
        index = int(found.group(1)) - 1
    fields = [name for name in body.get("fields", []) if isinstance(name, str)]
    reply = {
        "issues": [issue_entry(entries, key, fields) for key in pages[index]],
        "isLast": index == len(pages) - 1,
    }
    if index < len(pages) - 1:
        reply["nextPageToken"] = f"page-{index + 2}"
    return 200, reply


def route(entries, method, path, query, body):
    """`(status, payload)` for one request, before the timestamp pass."""
    if method == "GET" and path == "/rest/api/3/myself":
        return 200, entries["myself"]
    project = PROJECT_PATH.fullmatch(path)
    if method == "GET" and project:
        found = entries["projects"].get(project.group(1))
        if found is None:
            return 404, {"errorMessages": ["No project could be found with key"]}
        return 200, found
    if method == "POST" and path == "/rest/api/3/search/jql":
        return search(entries, body)
    issue = ISSUE_PATH.fullmatch(path)
    if method == "GET" and issue:
        key = issue.group(1)
        detail = entries["issue_details"].get(key)
        if detail is None:
            return 404, {
                "errorMessages": ["Issue does not exist or you do not have permission to see it"]
            }
        if issue.group(2):
            comments = detail.get("comments", [])
            return 200, {
                "startAt": 0,
                "maxResults": len(comments),
                "total": len(comments),
                "comments": comments,
            }
        return 200, issue_entry(entries, key, [])
    print(f"unmatched mock request: {method} {path}{query}", file=sys.stderr, flush=True)
    return 404, {"errorMessages": ["unsupported fixture request"]}


def load_fixture():
    try:
        entries = json.loads(FIXTURE.read_text())
    except (OSError, ValueError) as error:
        raise SystemExit(f"cannot read {FIXTURE}: {error}") from error
    for page in entries["search"]["pages"]:
        for key in page:
            if key not in entries["issue_details"]:
                raise SystemExit(f"{FIXTURE.name}: search key {key} has no issue_details entry")
    return entries


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--port-file", required=True)
    args = parser.parse_args()
    entries = load_fixture()
    lock = threading.Lock()

    class Handler(BaseHTTPRequestHandler):
        def log_message(self, format, *args):
            pass

        def handle_request(self):
            raw = self.rfile.read(int(self.headers.get("Content-Length", "0")))
            path = urlsplit(self.path)
            if authorized(self.headers.get("Authorization"), entries["auth"]):
                try:
                    body = json.loads(raw) if raw else {}
                except ValueError:
                    body = None
                status, payload = route(entries, self.command, path.path, path.query, body)
            else:
                status, payload = 401, {"errorMessages": ["You are not authenticated"]}
            with lock:
                print(f"mock {self.command} {path.path} {status}", flush=True)
            encoded = json.dumps(resolve_now(payload, datetime.now(UTC))).encode()
            self.send_response(status)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(encoded)))
            self.end_headers()
            self.wfile.write(encoded)

        do_GET = handle_request
        do_POST = handle_request

    server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    port_file = Path(args.port_file)
    temporary = port_file.with_name(port_file.name + ".tmp")
    temporary.write_text(str(server.server_port))
    temporary.replace(port_file)
    server.serve_forever()


if __name__ == "__main__":
    main()
