#!/usr/bin/env python3
"""Scratchpad dev server for the workbook UI (never committed).

Serves cli/assets/excel from the working tree at `/`, and answers `/api/*`
from ./fixtures (written by gen_fixtures.py from the design's §8.3 examples):
tiles are sliced from each sheet's cells with the contract's ETags and 304s;
`edge` computes Excel's Ctrl+arrow and current-region answers over the
fixture cells. Headers follow §8.2 (CSP, nosniff, no-referrer, no-store on
the API, the token header required). `POST edits`, `undo` and `redo` apply
§8.4's ops to the in-memory store (devstore.py): the revision moves, touched
tiles and changed layouts get new ETags, the change ring answers `changes`,
and a base behind an intersecting change is refused 409 `stale_base` (§8.6).

`summary` answers COUNTA, the numeric count, sum, average, min and max with
their text in the first numeric cell's format; `export` answers TSV (Excel's
quoting, CRLF rows) or an HTML table carrying `data-yggdryl-copy` with the
store's service `instance` (addendum 8); `POST calculate` recomputes every
formula and clears the uncomputed count. `find` answers addendum 10's next
match, `format` addendum 9's rendering of a code against a cell. `functions`
answers §5.6's 159 functions (functions_catalog.py); `POST formula/assist`
answers §8.3's three actions: `tokens` (the references in a formula, each
{start, end, sheet, range} with the range as `A1:B2`), `cycle` (F4: the `$`s
of the reference at the caret, relative -> absolute -> row -> column) and
`reference` (a pointed range spelled for the editing sheet, `Sheet2!B1:B3`
from another). The `replace` op takes the searched text as `text`
(addendum 11).

Test controls (no token): `POST /__control` with {"editLatency": ms} delays
every edit answer, {"latency": ms} every other API answer, {"holdChanges": true} keeps a long poll (`changes` with
`wait` > 0) from answering any change until released, {"closed": true}
answers `changes` 503 `closed` as a service after `close()` does; `GET /__requests` lists the requests seen, each with
its arrival and answer times (`at`, `end`).

Usage: dev.py [--port 0] [--token T] [--latency MS] [--edit-latency MS]
The first line printed is `listening on http://127.0.0.1:PORT/`.
"""
import argparse
import bisect
import json
import os
import pathlib
import re
import sys
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import parse_qs, unquote, urlsplit

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
from devstore import Refusal, Store, code_error, parse_range as store_range, ref_name as store_ref, render  # noqa: E402

HERE = pathlib.Path(__file__).resolve().parent
REPO = pathlib.Path(os.environ.get("YGGDRYL_REPO", pathlib.Path(__file__).resolve().parents[2]))
ASSETS = pathlib.Path(os.environ.get("YGG_ASSETS") or REPO / "cli" / "assets" / "excel")
FIXTURES = HERE / "fixtures"
SCRATCH = HERE.parent

MAX_ROWS = 1048576
MAX_COLUMNS = 16384
TILE_ROWS = 64
TILE_COLUMNS = 32

CSP = ("default-src 'self'; script-src 'self'; style-src 'self'; img-src 'self' data: blob:; "
       "connect-src 'self'; frame-ancestors 'none'; base-uri 'none'; form-action 'none'")
TYPES = {".html": "text/html; charset=utf-8", ".js": "text/javascript; charset=utf-8",
         ".css": "text/css; charset=utf-8", ".svg": "image/svg+xml", ".json": "application/json"}


def col_name(c):
    s = ""
    n = c + 1
    while n:
        n, r = divmod(n - 1, 26)
        s = chr(65 + r) + s
    return s


def ref_name(r, c):
    return f"{col_name(c)}{r + 1}"


REF = re.compile(r"^\$?([A-Za-z]{1,3})\$?([0-9]{1,7})$")


def parse_ref(text):
    m = REF.match(text.strip())
    if not m:
        return None
    c = 0
    for ch in m.group(1).upper():
        c = c * 26 + ord(ch) - 64
    r = int(m.group(2)) - 1
    c -= 1
    if not (0 <= r < MAX_ROWS and 0 <= c < MAX_COLUMNS):
        return None
    return r, c


def parse_range(text):
    parts = text.split(":")
    a = parse_ref(parts[0])
    b = parse_ref(parts[-1])
    if not a or not b:
        return None
    return min(a[0], b[0]), min(a[1], b[1]), max(a[0], b[0]), max(a[1], b[1])


class State:
    def __init__(self):
        self.store = Store(FIXTURES)
        self.closed = False
        self.requests = []
        self.lock = threading.RLock()


STATE = None
OPTIONS = None


class Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"
    server_version = "yggdryl-dev"

    def log_message(self, fmt, *args):
        if OPTIONS.verbose:
            sys.stderr.write("%s\n" % (fmt % args))

    def base_headers(self):
        self.send_header("X-Content-Type-Options", "nosniff")
        self.send_header("Referrer-Policy", "no-referrer")
        self.send_header("Content-Security-Policy", CSP)

    def answer(self, status, body=b"", content_type="application/json", headers=None, api=True):
        entry = getattr(self, "logged", None)
        if entry is not None:
            entry["status"] = status
            entry["end"] = time.time()
        self.send_response(status)
        self.base_headers()
        if api:
            self.send_header("Cache-Control", "no-store")
        for k, v in (headers or {}).items():
            self.send_header(k, v)
        if status != 304:
            self.send_header("Content-Type", content_type)
            self.send_header("Content-Length", str(len(body)))
        else:
            self.send_header("Content-Length", "0")
        try:
            self.end_headers()
            if self.command != "HEAD" and status != 304:
                self.wfile.write(body)
        except (BrokenPipeError, ConnectionResetError):
            # A long poll or a summary the page aborted: nobody is listening.
            self.close_connection = True

    def json(self, value, status=200, headers=None):
        self.answer(status, json.dumps(value).encode(), headers=headers)

    def problem(self, status, kind, detail, title=None, location=None):
        body = {"type": "about:blank", "title": title or {400: "Bad Request", 401: "Unauthorized", 403: "Forbidden",
                                                          404: "Not Found", 409: "Conflict", 415: "Unsupported Media Type",
                                                          422: "Unprocessable Content"}.get(status, "Error"),
                "status": status, "detail": detail, "kind": kind, "location": location,
                "revision": STATE.store.workbook["revision"]}
        self.answer(status, json.dumps(body).encode(), content_type="application/problem+json")

    def conditional(self, etag):
        tags = [t.strip() for t in (self.headers.get("If-None-Match") or "").split(",") if t.strip()]
        return etag in tags or "*" in tags

    def do_HEAD(self):
        self.do_GET()

    def do_GET(self):
        url = urlsplit(self.path)
        self.logged = {"method": self.command, "path": self.path, "ifNoneMatch": self.headers.get("If-None-Match"),
                       "token": self.headers.get("X-Yggdryl-Token"), "at": time.time()}
        with STATE.lock:
            STATE.requests.append(self.logged)
        if url.path.startswith("/api/"):
            return self.api(url)
        if url.path == "/__requests":
            with STATE.lock:
                value = list(STATE.requests)
            return self.json(value)
        return self.static(url.path)

    def do_POST(self):
        url = urlsplit(self.path)
        if url.path == "/__control":
            return self.control()
        length = int(self.headers.get("Content-Length") or 0)
        body = self.rfile.read(length) if length else b""
        self.logged = {"method": "POST", "path": self.path, "body": body.decode("utf-8", "replace")[:500], "at": time.time()}
        with STATE.lock:
            STATE.requests.append(self.logged)
        if not url.path.startswith("/api/"):
            return self.problem(405, "method", "only the API takes POST")
        if not self.checked():
            return
        if not (self.headers.get("Content-Type") or "").startswith("application/json"):
            return self.problem(415, "media_type", "expected application/json")
        path = url.path[len("/api/"):]
        try:
            payload = json.loads(body or b"null")
        except ValueError:
            return self.problem(400, "invalid_record", "the body is not JSON")
        store = STATE.store
        if path == "formula/assist":
            return self.assist(store, payload)
        if path in ("edits", "undo", "redo", "calculate"):
            delay = store.edit_latency or OPTIONS.edit_latency
            if delay:
                time.sleep(delay / 1000)
            with STATE.lock:
                try:
                    if path == "edits":
                        answer = store.edit(payload)
                    elif path == "calculate":
                        answer = store.calculate(payload)
                    else:
                        answer = store.history(payload, path)
                except Refusal as refusal:
                    return self.problem(refusal.status, refusal.kind, refusal.detail, location=refusal.location)
            return self.json(answer)
        return self.problem(422, "unsupported", "the dev server does not answer POST " + path)

    def control(self):
        length = int(self.headers.get("Content-Length") or 0)
        body = json.loads(self.rfile.read(length) or b"{}")
        with STATE.lock:
            if "editLatency" in body:
                STATE.store.edit_latency = int(body["editLatency"])
            if "latency" in body:
                OPTIONS.latency = int(body["latency"])
            if "holdChanges" in body:
                STATE.store.hold_changes = bool(body["holdChanges"])
            if "closed" in body:
                STATE.closed = bool(body["closed"])
            value = {"editLatency": STATE.store.edit_latency, "latency": OPTIONS.latency, "holdChanges": STATE.store.hold_changes,
                     "revision": STATE.store.workbook["revision"]}
        return self.json(value)

    def static(self, path):
        rel = unquote(path).lstrip("/") or "index.html"
        target = (ASSETS / rel).resolve()
        if ASSETS.resolve() not in target.parents and target != ASSETS.resolve():
            return self.answer(404, b"not found", "text/plain", api=False)
        if not target.is_file():
            return self.answer(404, b"not found", "text/plain", api=False)
        body = target.read_bytes()
        ctype = TYPES.get(target.suffix, "application/octet-stream")
        return self.answer(200, body, ctype, headers={"Cache-Control": "no-cache"}, api=False)

    def checked(self):
        origin = self.headers.get("Origin")
        host = self.headers.get("Host")
        if origin and origin != f"http://{host}":
            self.problem(403, "origin", f"origin {origin} is not the served origin")
            return False
        token = self.headers.get("X-Yggdryl-Token")
        if token is None:
            self.problem(401, "token", "every API request carries X-Yggdryl-Token")
            return False
        if OPTIONS.token is not None and token != OPTIONS.token:
            self.problem(401, "token", "the X-Yggdryl-Token does not match")
            return False
        return True

    def api(self, url):
        if not self.checked():
            return
        if OPTIONS.latency:
            time.sleep(OPTIONS.latency / 1000)
        path = url.path[len("/api/"):]
        query = {k: v[-1] for k, v in parse_qs(url.query).items()}
        if path == "changes":
            if STATE.closed:
                body = {"type": "about:blank", "title": "Service Unavailable", "status": 503,
                        "detail": "the workbook service has closed", "kind": "closed",
                        "revision": STATE.store.workbook["revision"]}
                return self.answer(503, json.dumps(body).encode(), content_type="application/problem+json")
            since = int(query.get("since", "0"))
            wait = min(float(query.get("wait", "0")), 2.0)
            deadline = time.time() + wait
            while True:
                with STATE.lock:
                    held = STATE.store.hold_changes and wait > 0
                    answer = STATE.store.changes(since)
                if held:
                    # Held: the feed says nothing new, whatever happened.
                    answer = {"revision": since, "changes": []}
                if answer.get("reset") or answer["changes"] or time.time() >= deadline:
                    return self.json(answer)
                time.sleep(0.05)
        with STATE.lock:
            return self.api_locked(path, query)

    def assist(self, store, body):
        """§8.3 `POST formula/assist`: tokens, cycle (F4) and reference (point mode)."""
        if OPTIONS.latency:
            time.sleep(OPTIONS.latency / 1000)
        if not isinstance(body, dict):
            return self.problem(400, "invalid_record", "$: expected an object")
        action = body.get("action")
        text = body.get("text")
        key = body.get("sheet")
        if action not in ("tokens", "cycle", "reference"):
            return self.problem(400, "invalid_record", f"$.action: expected tokens, cycle or reference, got {json.dumps(action)}")
        if not isinstance(text, str):
            return self.problem(400, "invalid_record", "$.text: expected the formula text")
        with STATE.lock:
            if key not in store.sheets:
                return self.problem(404, "not_found", f"no worksheet with key {json.dumps(key)}")
            names = {s["key"]: s["name"] for s in store.workbook["sheets"]}
            keys = {s["name"].lower(): s["key"] for s in store.workbook["sheets"]}
        if action == "tokens":
            return self.json({"tokens": [token for token, _ in formula_tokens(text, keys, key)]})
        if action == "cycle":
            caret = body.get("caret")
            if not isinstance(caret, int) or not 0 <= caret <= len(text):
                return self.problem(400, "invalid_record", f"$.caret: expected a position in the text, got {json.dumps(caret)}")
            for token, match in formula_tokens(text, keys, key):
                if token["start"] <= caret <= token["end"]:
                    spelled = cycled(match.group("ref"))
                    start = token["end"] - len(match.group("ref"))
                    out = text[:start] + spelled + text[token["end"]:]
                    return self.json({"text": out, "caret": start + len(spelled)})
            return self.json({"text": text, "caret": caret})
        wanted = body.get("range")
        if not isinstance(wanted, dict) or wanted.get("sheet") not in names:
            return self.problem(400, "invalid_record", "$.range: expected {sheet, range} naming a sheet")
        area = parse_range(wanted.get("range") or "")
        if area is None:
            return self.problem(400, "invalid_record", f"$.range.range: expected a range, got {json.dumps(wanted.get('range'))}")
        r0, c0, r1, c1 = area
        if r0 == 0 and r1 == MAX_ROWS - 1:
            spelled = f"{col_name(c0)}:{col_name(c1)}"
        elif c0 == 0 and c1 == MAX_COLUMNS - 1:
            spelled = f"{r0 + 1}:{r1 + 1}"
        elif (r0, c0) == (r1, c1):
            spelled = ref_name(r0, c0)
        else:
            spelled = f"{ref_name(r0, c0)}:{ref_name(r1, c1)}"
        if wanted["sheet"] != key:
            spelled = quote_sheet(names[wanted["sheet"]]) + "!" + spelled
        return self.json({"text": spelled})

    def find(self, store, query):
        """addendum 10: the next match after `from`, by rows, or null."""
        text = query.get("text", "")
        if not text:
            return self.problem(400, "invalid_record", "$.text: expected the text to find")
        scope = query.get("scope", "sheet")
        if scope not in ("sheet", "workbook"):
            return self.problem(400, "invalid_record", f"$.scope: expected sheet or workbook, got {scope!r}")
        within = query.get("in", "values")
        if within not in ("values", "formulas"):
            return self.problem(400, "invalid_record", f"$.in: expected values or formulas, got {within!r}")
        flags = {}
        for name in ("matchCase", "entireCell"):
            value = query.get(name, "false")
            if value not in ("true", "false"):
                return self.problem(400, "invalid_record", f"$.{name}: expected true or false, got {value!r}")
            flags[name] = value == "true"
        key = int(query["sheet"]) if query.get("sheet", "").isdigit() else None
        if key not in store.sheets:
            return self.problem(404, "not_found", f"no worksheet with key {query.get('sheet')}")
        origin = None
        if "from" in query:
            origin = parse_ref(query["from"])
            if origin is None:
                return self.problem(400, "invalid_record", f"$.from: expected a cell reference, got {query['from']!r}")
        return self.json(store.find(text, scope, key, origin, within, flags["matchCase"], flags["entireCell"]))

    def api_locked(self, path, query):
        store = STATE.store
        wb = store.document()
        if path == "workbook":
            return self.json(wb)
        if path == "styles":
            start = int(query.get("from", "0"))
            return self.json({"count": len(store.styles), "styles": store.styles[start:]})
        if path == "functions":
            return self.json(store.functions)
        if path == "media":
            return self.json(store.media)
        if path == "workbook/download":
            source = SCRATCH / "styled.xlsx"
            body = source.read_bytes() if source.exists() else b"PK\x05\x06" + b"\0" * 18
            return self.answer(200, body, "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
                               headers={"Content-Disposition": "attachment; filename=\"book.xlsx\"; filename*=UTF-8''book.xlsx"})
        if path == "find":
            return self.find(store, query)
        if path == "format":
            code = query.get("code")
            if code is None:
                return self.problem(400, "invalid_record", "format takes code=<number format code>")
            error = code_error(code)
            if error:
                return self.problem(400, "invalid_record", f"$.code: {error}")
            key = int(query["sheet"]) if query.get("sheet", "").isdigit() else None
            at = parse_ref(query.get("ref", ""))
            if key not in store.sheets:
                return self.problem(404, "not_found", f"no worksheet with key {query.get('sheet')}")
            if not at:
                return self.problem(400, "invalid_record", "format takes ref=A1")
            return self.json(store.format_preview(code, key, at))
        if path == "resolve":
            text = query.get("text", "").strip()
            name = next((n for n in wb["names"] if n["name"].lower() == text.lower()), None)
            if name:
                sheet, rng = name["text"].lstrip("=").split("!")
                key = next(s["key"] for s in wb["sheets"] if s["name"] == sheet)
                return self.json({"sheet": key, "range": rng.replace("$", "")})
            sheet_key = None
            if "!" in text:
                sheet_name, text = text.rsplit("!", 1)
                sheet_key = next((s["key"] for s in wb["sheets"] if s["name"].lower() == sheet_name.strip("'").lower()), None)
                if sheet_key is None:
                    return self.problem(400, "invalid_reference", f"there is no sheet named '{sheet_name.strip(chr(39))}'")
            rng = store_range(text.replace("$", ""))
            if not rng:
                return self.problem(400, "invalid_reference", f"'{text}' is not a reference or a name")
            r0, c0, r1, c1 = rng
            out = store_ref(r0, c0) if (r0, c0) == (r1, c1) else f"{store_ref(r0, c0)}:{store_ref(r1, c1)}"
            return self.json({"sheet": sheet_key, "range": out})
        m = re.fullmatch(r"sheets/(\d+)/(.+)", path)
        if not m:
            return self.problem(404, "not_found", f"no API at {path}")
        key = int(m.group(1))
        sheet = store.sheets.get(key)
        if sheet is None:
            return self.problem(404, "not_found", f"no worksheet with key {key}")
        rest = m.group(2)
        if rest == "layout":
            etag = store.layout_etag(key)
            if self.conditional(etag):
                return self.answer(304, headers={"ETag": etag})
            return self.json(sheet.layout, headers={"ETag": etag})
        t = re.fullmatch(r"tiles/(\d+)/(\d+)", rest)
        if t:
            tr, tc = int(t.group(1)), int(t.group(2))
            if tr >= MAX_ROWS // TILE_ROWS or tc >= MAX_COLUMNS // TILE_COLUMNS:
                return self.problem(400, "invalid_reference", f"tile {tr}/{tc} is outside the sheet")
            etag = store.tile_etag(key, tr, tc)
            if self.conditional(etag):
                return self.answer(304, headers={"ETag": etag})
            return self.json(sheet.tile(tr, tc), headers={"ETag": etag})
        c = re.fullmatch(r"cells/([A-Za-z]{1,3}[0-9]{1,7})", rest)
        if c:
            at = parse_ref(c.group(1))
            if not at:
                return self.problem(400, "invalid_reference", f"'{c.group(1)}' is not a cell reference")
            return self.json(sheet.cell_doc(*at))
        if rest == "edge":
            at = parse_ref(query.get("from", ""))
            direction = query.get("direction", "")
            if not at or direction not in ("up", "down", "left", "right", "region"):
                return self.problem(400, "invalid_record", "edge takes from=A1 and direction=up|down|left|right|region")
            if direction == "region":
                r0, c0, r1, c1 = sheet.region(*at)
                text = ref_name(r0, c0) if (r0, c0) == (r1, c1) else f"{ref_name(r0, c0)}:{ref_name(r1, c1)}"
                return self.json({"range": text})
            r, cc = sheet.edge(at[0], at[1], direction)
            return self.json({"ref": ref_name(r, cc)})
        if rest == "summary":
            return self.json(summary(store, sheet, query.get("range", "")))
        if rest == "export":
            g = parse_range(query.get("range", ""))
            fmt = query.get("format", "tsv")
            if not g or fmt not in ("tsv", "html"):
                return self.problem(400, "invalid_record", "export takes range=A1:C3 and format=tsv|html")
            if (g[2] - g[0] + 1) * (g[3] - g[1] + 1) > 1_000_000:
                return self.problem(413, "too_large", f"{sheet_name(store, key)}!{query['range']}: expected at most "
                                                      f"1000000 cells in an export, got {(g[2] - g[0] + 1) * (g[3] - g[1] + 1)}",
                                    title="Content Too Large")
            rows = [[(sheet.cells.get((r, c)) or {}).get("text", "") for c in range(g[1], g[3] + 1)]
                    for r in range(g[0], g[2] + 1)]
            if fmt == "tsv":
                return self.answer(200, tsv(rows).encode(), "text/tab-separated-values; charset=utf-8")
            marker = f"{wb['instance']}:{wb['generation']}:{wb['revision']}:{key}:{query['range']}"
            return self.answer(200, html_table(rows, marker).encode(), "text/html; charset=utf-8")
        return self.problem(404, "not_found", f"no API at {path}")


# A formula's references: an optional sheet, then a cell, an area, columns or
# rows, never part of a longer name or a call.
REFERENCE = re.compile(r"""(?<![\w$.!'"])
  (?P<sheet>(?:'(?:[^']|'')+'|[A-Za-z_][\w.]*)!)?
  (?P<ref>\$?[A-Za-z]{1,3}\$?\d{1,7}(?::\$?[A-Za-z]{1,3}\$?\d{1,7})?
        |\$?[A-Za-z]{1,3}:\$?[A-Za-z]{1,3}
        |\$?\d{1,7}:\$?\d{1,7})
  (?![\w(!])""", re.X)


def formula_tokens(text, keys, own):
    """([{start, end, sheet, range}], match) for each reference outside strings."""
    out = []
    if not text.startswith("="):
        return out
    at = 0
    for part in re.split(r'("(?:[^"]|"")*"?)', text):
        if not part.startswith('"'):
            for m in REFERENCE.finditer(part):
                sheet = own
                if m.group("sheet"):
                    name = m.group("sheet")[:-1]
                    if name.startswith("'"):
                        name = name[1:-1].replace("''", "'")
                    sheet = keys.get(name.lower())
                ref = m.group("ref").replace("$", "")
                first, _, second = ref.partition(":")
                second = second or first
                if first.isdigit():
                    first, second = "A" + first, "XFD" + second
                elif first.isalpha():
                    first, second = first + "1", second + str(MAX_ROWS)
                area = parse_range(first + ":" + second)
                if area is None:
                    continue
                r0, c0, r1, c1 = area
                out.append(({"start": at + m.start(), "end": at + m.end(), "sheet": sheet,
                             "range": f"{ref_name(r0, c0)}:{ref_name(r1, c1)}"}, m))
        at += len(part)
    return out


def cycled(ref):
    """F4 on one reference: A1 -> $A$1 -> A$1 -> $A1 -> A1, both corners alike."""
    parts = ref.split(":")
    head = parts[0]
    if head.lstrip("$").isdigit() or head.lstrip("$").isalpha():
        absolute = head.startswith("$")
        return ":".join(("" if absolute else "$") + p.lstrip("$") for p in parts)
    m = re.match(r"(\$?)([A-Za-z]{1,3})(\$?)(\d+)", head)
    state = (bool(m.group(1)), bool(m.group(3)))
    order = [(False, False), (True, True), (False, True), (True, False)]
    column, row = order[(order.index(state) + 1) % 4]

    def spell(corner):
        c = re.match(r"\$?([A-Za-z]{1,3})\$?(\d+)", corner)
        return ("$" if column else "") + c.group(1) + ("$" if row else "") + c.group(2)
    return ":".join(spell(p) for p in parts)


def quote_sheet(name):
    return name if re.fullmatch(r"[A-Za-z_][\w.]*", name) else "'" + name.replace("'", "''") + "'"


def sheet_name(store, key):
    return next(s["name"] for s in store.workbook["sheets"] if s["key"] == key)


def summary(store, sheet, ranges):
    """§8.3 summary: COUNTA, numbers, sum, average, min, max; text in the first number's format."""
    count = numbers = 0
    total = 0.0
    low = high = None
    code = None
    seen = set()
    for text in ranges.split(","):
        g = parse_range(text)
        if not g:
            continue
        for (r, c) in sorted(sheet.cells, key=lambda p: (p[0], p[1])):
            if not (g[0] <= r <= g[2] and g[1] <= c <= g[3]) or (r, c) in seen:
                continue
            seen.add((r, c))
            item = sheet.cells[(r, c)]
            if item["text"] == "":
                continue
            count += 1
            if item["flags"] & 3 == 1:
                value = sheet.value_at(r, c)
                value = float(value) if isinstance(value, (int, float)) else 0.0
                numbers += 1
                total += value
                low = value if low is None else min(low, value)
                high = value if high is None else max(high, value)
                if code is None:
                    code = store.code_of(item["style"])
    if numbers == 0:
        return {"count": count, "numbers": 0, "sum": 0, "average": None, "min": None, "max": None, "text": None}
    average = total / numbers
    shown = lambda v: render(v, code)[0].strip()  # noqa: E731
    return {"count": count, "numbers": numbers, "sum": total, "average": average, "min": low, "max": high,
            "text": {"sum": shown(total), "average": shown(average), "min": shown(low), "max": shown(high)}}


def tsv(rows):
    """Excel's text form: tabs, CRLF rows, a cell holding a tab, a line break or a quote quoted."""
    def cell(text):
        return '"' + text.replace('"', '""') + '"' if any(ch in text for ch in '\t\n\r"') else text
    return "".join("\t".join(cell(t) for t in row) + "\r\n" for row in rows)


def html_table(rows, marker):
    def esc(text):
        return text.replace("&", "&amp;").replace("<", "&lt;").replace(">", "&gt;").replace('"', "&quot;")
    body = "".join("<tr>" + "".join(f"<td>{esc(t)}</td>" for t in row) + "</tr>" for row in rows)
    return (f'<html><head><meta charset="utf-8"></head><body><table data-yggdryl-copy="{marker}">'
            f"{body}</table></body></html>")


def main():
    global STATE, OPTIONS
    parser = argparse.ArgumentParser()
    parser.add_argument("--port", type=int, default=0)
    parser.add_argument("--token", default=None)
    parser.add_argument("--latency", type=int, default=0, help="milliseconds added to each API answer")
    parser.add_argument("--edit-latency", type=int, default=0, help="milliseconds added to each edit answer")
    parser.add_argument("--verbose", action="store_true")
    OPTIONS = parser.parse_args()
    STATE = State()
    server = ThreadingHTTPServer(("127.0.0.1", OPTIONS.port), Handler)
    server.daemon_threads = True
    print(f"listening on http://127.0.0.1:{server.server_address[1]}/", flush=True)
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        pass


if __name__ == "__main__":
    main()
