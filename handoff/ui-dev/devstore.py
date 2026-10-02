"""The dev server's workbook store (scratchpad, never committed).

Holds the fixture workbook in memory and answers §8.4's edit ops against it
plausibly: en-US typed entry (a subset of §3.5), a small formula evaluator,
number-format rendering good enough to see the ribbon work (General, fixed,
thousands, currency, accounting with its `*` fill, percent, scientific,
fractions, dates and times), style interning, the structural and layout ops,
sheets, and a snapshot journal for undo/redo. Revisions, per-tile revisions,
the change ring and the §8.6 stale-base rule follow the contract. P4: the
`replace` op (its searched text `text`, addendum 11), `find` (addendum 10)
and `format` (addendum 9) answers, and the service `instance` a copy marker
names (addendum 8).
"""
import bisect
import copy
import datetime
import json
import math
import os
import re
import time
import uuid

MAX_ROWS = 1048576
MAX_COLUMNS = 16384
TILE_ROWS = 64
TILE_COLUMNS = 32
DEFAULT_WIDTH = 9.140625
RING = 1024


def uuid7():
    """A UUIDv7: 48 bits of Unix milliseconds, version 7, the rest random."""
    ms = int(time.time() * 1000) & ((1 << 48) - 1)
    rand = int.from_bytes(os.urandom(10), "big")
    value = ms << 80 | 0x7 << 76 | ((rand >> 62) & 0xFFF) << 64 | 0b10 << 62 | (rand & ((1 << 62) - 1))
    return str(uuid.UUID(int=value))


def wildcard(text, match_case, entire):
    """§4.3's criteria wildcards: `*` any run, `?` one character, `~` escapes the next."""
    out, i = "", 0
    while i < len(text):
        ch = text[i]
        if ch == "~" and i + 1 < len(text):
            out += re.escape(text[i + 1])
            i += 2
            continue
        out += ".*" if ch == "*" else "." if ch == "?" else re.escape(ch)
        i += 1
    flags = re.DOTALL | (0 if match_case else re.IGNORECASE)
    return re.compile("^(?:" + out + ")$" if entire else out, flags)


def code_error(code):
    """Why a number-format code cannot be read, naming the byte, or None."""
    data = code.encode("utf-8")
    i, sections = 0, 1
    while i < len(data):
        ch = data[i:i + 1]
        if ch == b'"':
            end = data.find(b'"', i + 1)
            if end < 0:
                return f"expected a closing quote for the one at byte {i}"
            i = end + 1
            continue
        if ch == b"[":
            end = data.find(b"]", i + 1)
            if end < 0:
                return f"expected ] closing the [ at byte {i}"
            i = end + 1
            continue
        if ch in (b"\\", b"_", b"*"):
            if i + 1 >= len(data):
                return f"expected a character after {ch.decode()} at byte {i}"
            i += 2
            continue
        if ch == b";":
            sections += 1
            if sections > 4:
                return f"expected at most 4 sections, got a fifth at byte {i}"
        i += 1
    return None


class Refusal(Exception):
    def __init__(self, status, kind, detail, location=None):
        super().__init__(detail)
        self.status = status
        self.kind = kind
        self.detail = detail
        self.location = location


def col_name(c):
    s = ""
    n = c + 1
    while n:
        n, r = divmod(n - 1, 26)
        s = chr(65 + r) + s
    return s


def col_index(letters):
    n = 0
    for ch in letters.upper():
        n = n * 26 + ord(ch) - 64
    return n - 1


def ref_name(r, c):
    return f"{col_name(c)}{r + 1}"


def range_name(g):
    r0, c0, r1, c1 = g
    return ref_name(r0, c0) if (r0, c0) == (r1, c1) else f"{ref_name(r0, c0)}:{ref_name(r1, c1)}"


REF = re.compile(r"^\$?([A-Za-z]{1,3})\$?([0-9]{1,7})$")
COLS = re.compile(r"^\$?([A-Za-z]{1,3})$")
ROWS = re.compile(r"^\$?([0-9]{1,7})$")


def parse_ref(text):
    m = REF.match(text.strip())
    if not m:
        return None
    c = col_index(m.group(1))
    r = int(m.group(2)) - 1
    if not (0 <= r < MAX_ROWS and 0 <= c < MAX_COLUMNS):
        return None
    return r, c


def parse_range(text):
    parts = text.split(":")
    if len(parts) > 2:
        return None
    first, second = parts[0], parts[-1]
    a, b = parse_ref(first), parse_ref(second)
    if a and b:
        return min(a[0], b[0]), min(a[1], b[1]), max(a[0], b[0]), max(a[1], b[1])
    ca, cb = COLS.match(first.strip()), COLS.match(second.strip())
    if ca and cb:
        x, y = col_index(ca.group(1)), col_index(cb.group(1))
        return 0, min(x, y), MAX_ROWS - 1, max(x, y)
    ra, rb = ROWS.match(first.strip()), ROWS.match(second.strip())
    if ra and rb:
        x, y = int(ra.group(1)) - 1, int(rb.group(1)) - 1
        return min(x, y), 0, max(x, y), MAX_COLUMNS - 1
    return None


def intersects(a, b):
    return a[0] <= b[2] and b[0] <= a[2] and a[1] <= b[3] and b[1] <= a[3]


def area(g):
    return (g[2] - g[0] + 1) * (g[3] - g[1] + 1)


# Number formats ---------------------------------------------------------------

COLORS = {"black": "#000000", "blue": "#0000FF", "cyan": "#00FFFF", "green": "#00FF00",
          "magenta": "#FF00FF", "red": "#FF0000", "white": "#FFFFFF", "yellow": "#FFFF00"}
BUILTIN = {0: "General", 1: "0", 2: "0.00", 3: "#,##0", 4: "#,##0.00", 9: "0%", 10: "0.00%", 11: "0.00E+00",
           12: "# ?/?", 13: "# ??/??", 14: "m/d/yyyy", 18: "h:mm AM/PM", 19: "h:mm:ss AM/PM", 20: "h:mm",
           21: "h:mm:ss", 22: "m/d/yyyy h:mm", 49: "@"}


def split_sections(code):
    out, cur, quoted, bracket, escape = [], "", False, False, False
    for ch in code:
        if escape:
            cur += ch
            escape = False
            continue
        if ch == "\\" and not quoted:
            cur += ch
            escape = True
            continue
        if ch == '"':
            quoted = not quoted
        elif ch == "[" and not quoted:
            bracket = True
        elif ch == "]" and not quoted:
            bracket = False
        if ch == ";" and not quoted and not bracket:
            out.append(cur)
            cur = ""
            continue
        cur += ch
    out.append(cur)
    return out


def bare(section):
    """The section with quoted text, escapes and brackets removed."""
    return re.sub(r'"[^"]*"|\\.|\[[^\]]*\]|_.|\*.', "", section)


def general(value):
    if isinstance(value, bool):
        return "TRUE" if value else "FALSE", None
    if value == int(value) and abs(value) < 1e11:
        text = str(int(value))
        return text, None
    text = None
    for p in range(15, 0, -1):
        s = f"{value:.{p}g}"
        if "e" in s:
            mant, exp = s.split("e")
            s = f"{mant}E{'+' if int(exp) >= 0 else '-'}{abs(int(exp)):02d}"
        if len(s) <= 11:
            text = s
            break
    text = text or f"{value:.2E}"
    shorter = None
    if len(text) > 8:
        shorter = []
        for p in (5, 3, 1):
            s = f"{value:.{p}g}"
            if "e" not in s and s != text and s not in shorter:
                shorter.append(s)
    return text, shorter


def serial_to_datetime(serial):
    base = datetime.datetime(1899, 12, 30)
    if serial < 61:
        base = datetime.datetime(1899, 12, 31)
    return base + datetime.timedelta(days=serial)


def datetime_to_serial(moment):
    delta = moment - datetime.datetime(1899, 12, 30)
    return delta.days + delta.seconds / 86400


def render_date(value, section):
    moment = serial_to_datetime(value)
    out = ""
    tokens = re.findall(r'"[^"]*"|\\.|\[[^\]]*\]|yyyy|yy|mmmmm|mmmm|mmm|mm|m|dddd|ddd|dd|d|hh|h|ss|s|AM/PM|am/pm|A/P|.', section)
    has_ampm = any(t.upper() in ("AM/PM", "A/P") for t in tokens)
    last_hour = False
    for i, tok in enumerate(tokens):
        low = tok.lower()
        if tok.startswith('"'):
            out += tok[1:-1]
        elif tok.startswith("\\"):
            out += tok[1:]
        elif tok.startswith("["):
            continue
        elif low == "yyyy":
            out += f"{moment.year:04d}"
        elif low == "yy":
            out += f"{moment.year % 100:02d}"
        elif low in ("mm", "m"):
            following = [t.lower() for t in tokens[i + 1:i + 3]]
            minute = last_hour or any(t in ("ss", "s") for t in following)
            value_ = moment.minute if minute else moment.month
            out += f"{value_:02d}" if low == "mm" else str(value_)
        elif low == "mmm":
            out += moment.strftime("%b")
        elif low == "mmmm":
            out += moment.strftime("%B")
        elif low == "mmmmm":
            out += moment.strftime("%B")[0]
        elif low == "dddd":
            out += moment.strftime("%A")
        elif low == "ddd":
            out += moment.strftime("%a")
        elif low == "dd":
            out += f"{moment.day:02d}"
        elif low == "d":
            out += str(moment.day)
        elif low in ("hh", "h"):
            hour = moment.hour % 12 or 12 if has_ampm else moment.hour
            out += f"{hour:02d}" if low == "hh" else str(hour)
            last_hour = True
            continue
        elif low in ("ss", "s"):
            out += f"{moment.second:02d}" if low == "ss" else str(moment.second)
        elif tok.upper() == "AM/PM":
            out += "AM" if moment.hour < 12 else "PM"
        elif tok.upper() == "A/P":
            out += "A" if moment.hour < 12 else "P"
        else:
            out += tok
        if low not in (" ", ":"):
            last_hour = False
    return out


NUMBER_RUN = r"[0#?][0#?,]*(?:\.[0#?]*)?(?:E[+-]0+)?|\.[0#?]+"
SECTION_TOKENS = re.compile(r'"[^"]*"|\\.|\[[^\]]*\]|_.|\*.|' + NUMBER_RUN + r'|@|.')
FRACTION = re.compile(r"[#0?]+\s+[#0?]+/[#0?0-9]+|[#0?]+/[#0?0-9]+")


def render_number(value, section, negative_sign):
    """One section: literals, `_x` spaces, one `*x` fill, one numeric run."""
    color = None
    fill = None
    out = ""
    scaled = value * 100 if "%" in bare(section) else value
    fraction = FRACTION.search(bare(section))
    used = False
    for tok in SECTION_TOKENS.findall(section):
        if tok.startswith('"'):
            out += tok[1:-1]
        elif tok.startswith("\\"):
            out += tok[1:]
        elif tok.startswith("["):
            color = COLORS.get(tok[1:-1].lower(), color)
        elif tok.startswith("_"):
            out += " "
        elif tok.startswith("*"):
            fill = [tok[1], len(out)]
        elif tok == "@":
            out += general(value)[0]
        elif re.match(r"^(?:[0#?]|\.[0#?])", tok):
            if not used:
                out += number_run(scaled, fraction.group(0) if fraction else tok)
                used = True
        elif fraction and tok in "/ " and used:
            continue
        else:
            out += tok
    if negative_sign:
        out = "-" + out
    return out, fill, color


def number_run(value, run):
    if "/" in run:
        whole = int(abs(value))
        frac = abs(value) - whole
        digits = len(re.sub(r"[^?0-9#]", "", run.split("/")[1]))
        best = (0, 1)
        for d in range(1, 10 ** digits):
            n = round(frac * d)
            if abs(frac - n / d) < abs(frac - best[0] / max(1, best[1])) - 1e-12:
                best = (n, d)
        n, d = best
        if n == d:
            whole, n = whole + 1, 0
        text = str(whole) if whole or not n else ""
        if n:
            text = (text + " " if text else "") + f"{n}/{d}"
        return text or "0"
    exp = re.search(r"E([+-])(0+)$", run)
    if exp:
        mant = run[:exp.start()]
        decimals = len(mant.split(".")[1]) if "." in mant else 0
        s = f"{value:.{decimals}E}"
        m, e = s.split("E")
        return f"{m}E{'+' if int(e) >= 0 else '-'}{abs(int(e)):0{len(exp.group(2))}d}"
    if set(run) <= {"?"}:
        return " " * len(run)
    head, _, tail = run.partition(".")
    decimals = len(re.sub(r"[^0#?]", "", tail))
    thousands = "," in head.strip(",")
    body = f"{value:,.{decimals}f}" if thousands else f"{value:.{decimals}f}"
    if head.replace(",", "") and set(head.replace(",", "")) <= {"#"} and body.startswith("0") and value < 1:
        body = body[1:]
    return body


def render(value, code):
    """A number under a format code: (text, fill, color, shorter)."""
    if code in (None, "", "General"):
        text, shorter = general(value)
        return text, None, None, shorter
    sections = split_sections(code)
    negative_sign = None
    if value < 0 and len(sections) >= 2:
        section = sections[1]
        value_ = -value
        negative_sign = False
    elif value == 0 and len(sections) >= 3:
        section = sections[2]
        value_ = value
    else:
        section = sections[0]
        value_ = value
        if value < 0:
            negative_sign = True
            value_ = -value
    if bare(section).lower().replace("general", "") == "" and "general" in section.lower():
        text, shorter = general(value_)
        return ("-" if negative_sign else "") + text, None, None, shorter
    if is_date_section(section):
        return render_date(value, section), None, None, None
    text, fill, color = render_number(value_, section, negative_sign)
    return text, fill, color, None


def is_date_section(section):
    b = bare(section).lower()
    return bool(re.search(r"[ydhs]", b) or re.search(r"(^|[^0#?])m", b)) and not re.search(r"[0#?]", b.replace("ss.0", "").replace("s.0", ""))


def render_text(text, code):
    sections = split_sections(code or "General")
    section = sections[3] if len(sections) >= 4 else sections[0]
    if "@" not in section:
        return text, None
    out, fill = "", None
    for tok in re.findall(r'"[^"]*"|\\.|\[[^\]]*\]|_.|\*.|@|.', section):
        if tok.startswith('"'):
            out += tok[1:-1]
        elif tok.startswith("\\"):
            out += tok[1:]
        elif tok.startswith("["):
            continue
        elif tok.startswith("_"):
            out += " "
        elif tok.startswith("*"):
            fill = [tok[1], len(out)]
        elif tok == "@":
            out += text
        else:
            out += tok
    return out, fill


def with_decimals(code, delta, value=None):
    if code in (None, "", "General"):
        if delta < 0:
            return "General"
        decimals = 0
        if value is not None and value != int(value):
            decimals = len(general(value)[0].split(".")[-1])
        return "0." + "0" * (decimals + 1) if decimals + 1 > 0 else "0"
    out = []
    for section in split_sections(code):
        pieces = re.split(r'("[^"]*"|\\.|\[[^\]]*\]|_.|\*.)', section)
        done = False
        for i, piece in enumerate(pieces):
            if done or i % 2 == 1:
                continue
            m = re.search(r"[0#][0#?,]*(\.[0#?]*)?", piece)
            if not m:
                continue
            run = m.group(0)
            if delta > 0:
                new = run + ("0" if "." in run else ".0")
            else:
                if "." not in run:
                    continue
                new = run[:-1]
                if new.endswith("."):
                    new = new[:-1]
            pieces[i] = piece[:m.start()] + new + piece[m.end():]
            done = True
        out.append("".join(pieces))
    return ";".join(out)


# Typed entry ------------------------------------------------------------------

ERRORS = ["#NULL!", "#DIV/0!", "#VALUE!", "#REF!", "#NAME?", "#NUM!", "#N/A", "#GETTING_DATA", "#SPILL!",
          "#CALC!", "#FIELD!", "#BLOCKED!", "#CONNECT!", "#BUSY!", "#UNKNOWN!", "#EXTERNAL!", "#PYTHON!", "#TIMEOUT!"]


def parse_entry(text):
    """(kind, value, suggested format) for typed text; kind: text, number, boolean, error, formula, blank."""
    if text == "":
        return "blank", None, None
    if text.startswith("="):
        return "formula", text, None
    if text.startswith("'"):
        return "text", text[1:], None
    upper = text.strip().upper()
    if upper in ("TRUE", "FALSE"):
        return "boolean", upper == "TRUE", None
    if upper in ERRORS:
        return "error", upper, None
    s = text.strip()
    if re.fullmatch(r"[+-]?[a-zA-Z(].*", s) and s[0] in "+-" and not re.fullmatch(r"[+-]?\(?\$?[\d,.]+\)?%?", s):
        return "formula", "=" + s, None
    m = re.fullmatch(r"([+-]?)((?:\d{1,3}(?:,\d{3})+|\d*)(?:\.\d+)?)%", s)
    if m and m.group(2):
        decimals = len(m.group(2).split(".")[1]) if "." in m.group(2) else 0
        value = float(m.group(1) + m.group(2).replace(",", "")) / 100
        return "number", value, "0.00%" if decimals else "0%"
    m = re.fullmatch(r"(\()?([+-]?)\$((?:\d{1,3}(?:,\d{3})+|\d+)(?:\.\d+)?|\.\d+)(\))?", s)
    if m and bool(m.group(1)) == bool(m.group(4)):
        value = float(m.group(3).replace(",", ""))
        if m.group(2) == "-" or m.group(1):
            value = -value
        return "number", value, '"$"#,##0.00'
    m = re.fullmatch(r"(\()?([+-]?(?:\d{1,3}(?:,\d{3})+|\d+)?(?:\.\d+)?)(\))?", s)
    if m and m.group(2) not in ("", "+", "-") and bool(m.group(1)) == bool(m.group(3)):
        value = float(m.group(2).replace(",", ""))
        if m.group(1):
            value = -value
        return "number", value, "#,##0" if "," in m.group(2) and "." not in m.group(2) else ("#,##0.00" if "," in m.group(2) else None)
    m = re.fullmatch(r"[+-]?\d+(?:\.\d+)?[eE][+-]?\d+", s)
    if m:
        return "number", float(s), "0.00E+00"
    m = re.fullmatch(r"(\d+) (\d+)/(\d+)", s)
    if m and int(m.group(3)):
        return "number", int(m.group(1)) + int(m.group(2)) / int(m.group(3)), "# ?/?"
    m = re.fullmatch(r"(\d{1,2})/(\d{1,2})/(\d{2,4})(?: (\d{1,2}):(\d{2}))?", s)
    if m:
        year = int(m.group(3))
        year += 2000 if year < 30 else (1900 if year < 100 else 0)
        try:
            moment = datetime.datetime(year, int(m.group(1)), int(m.group(2)),
                                       int(m.group(4) or 0), int(m.group(5) or 0))
        except ValueError:
            return "text", text, None
        return "number", datetime_to_serial(moment), "m/d/yyyy h:mm" if m.group(4) else "m/d/yyyy"
    m = re.fullmatch(r"(\d{4})-(\d{1,2})-(\d{1,2})", s)
    if m:
        try:
            moment = datetime.datetime(int(m.group(1)), int(m.group(2)), int(m.group(3)))
        except ValueError:
            return "text", text, None
        return "number", datetime_to_serial(moment), "m/d/yyyy"
    m = re.fullmatch(r"(\d{1,2}):(\d{2})(?::(\d{2}))?(?:\s*([AaPp][Mm]))?", s)
    if m:
        hour = int(m.group(1))
        if m.group(4):
            hour = hour % 12 + (12 if m.group(4).lower() == "pm" else 0)
        value = (hour * 3600 + int(m.group(2)) * 60 + int(m.group(3) or 0)) / 86400
        return "number", value, "h:mm AM/PM" if m.group(4) else ("h:mm:ss" if m.group(3) else "h:mm")
    return "text", text, None


# Formulas ---------------------------------------------------------------------

TOKEN = re.compile(r"""\s*(?:
    (?P<string>"(?:[^"]|"")*")
  | (?P<range>(?:(?:'[^']+'|[A-Za-z_][\w.]*)!)?\$?[A-Za-z]{1,3}\$?\d{1,7}:\$?[A-Za-z]{1,3}\$?\d{1,7})
  | (?P<ref>(?:(?:'[^']+'|[A-Za-z_][\w.]*)!)?\$?[A-Za-z]{1,3}\$?\d{1,7}(?![\w(]))
  | (?P<func>[A-Za-z][A-Za-z0-9.]*)\s*\(
  | (?P<bool>TRUE|FALSE)(?![\w(])
  | (?P<number>\d+(?:\.\d*)?(?:[eE][+-]?\d+)?|\.\d+)
  | (?P<op><>|<=|>=|[-+*/^&=<>(),%:])
  | (?P<name>[A-Za-z_][\w.]*)
)""", re.X)


class FormulaError(Exception):
    def __init__(self, message, position):
        super().__init__(message)
        self.position = position


def tokenize(formula):
    body = formula[1:]
    at = 0
    out = []
    depth = 0
    while at < len(body):
        if body[at:].strip() == "":
            break
        m = TOKEN.match(body, at)
        if not m or m.end() == at:
            raise FormulaError(f"unexpected '{body[at:].strip()[:1]}'", at + 1)
        kind = m.lastgroup
        value = m.group(kind)
        if kind == "func":
            depth += 1
        elif kind == "op" and value == "(":
            depth += 1
        elif kind == "op" and value == ")":
            depth -= 1
            if depth < 0:
                raise FormulaError("a ')' closes nothing", m.start(kind) + 1)
        out.append((kind, value, m.start(kind) + 1))
        at = m.end()
    if depth > 0:
        raise FormulaError("expected ')'", len(formula))
    if not out:
        raise FormulaError("expected an expression after '='", 1)
    return out


def shift_formula(formula, dr, dc):
    """Relative references moved by (dr, dc), as a copy or a fill does."""
    def move(m):
        col_abs, col, row_abs, row = m.group(1), m.group(2), m.group(3), m.group(4)
        c = col_index(col) + (0 if col_abs else dc)
        r = int(row) - 1 + (0 if row_abs else dr)
        if not (0 <= r < MAX_ROWS and 0 <= c < MAX_COLUMNS):
            return "#REF!"
        return f"{col_abs}{col_name(c)}{row_abs}{r + 1}"
    parts = re.split(r'("(?:[^"]|"")*")', formula)
    for i in range(0, len(parts), 2):
        parts[i] = re.sub(r"(?<![\w$])(\$?)([A-Za-z]{1,3})(\$?)(\d{1,7})(?![\w(])", move, parts[i])
    return "".join(parts)


class Evaluator:
    FUNCTIONS = {
        "SUM": lambda *a: sum(x for x in flat(a) if isinstance(x, (int, float)) and not isinstance(x, bool)),
        "AVERAGE": lambda *a: mean([x for x in flat(a) if isinstance(x, (int, float)) and not isinstance(x, bool)]),
        "COUNT": lambda *a: len([x for x in flat(a) if isinstance(x, (int, float)) and not isinstance(x, bool)]),
        "COUNTA": lambda *a: len([x for x in flat(a) if x is not None and x != ""]),
        "MAX": lambda *a: max([x for x in flat(a) if isinstance(x, (int, float))] or [0]),
        "MIN": lambda *a: min([x for x in flat(a) if isinstance(x, (int, float))] or [0]),
        "ROUND": lambda x, d=0: round(num(x), int(num(d))),
        "ABS": lambda x: abs(num(x)),
        "IF": lambda c, a=True, b=False: a if c else b,
        "CONCATENATE": lambda *a: "".join(str(x) for x in flat(a)),
        "TODAY": lambda: float(int(datetime_to_serial(datetime.datetime.now()))),
        "NOW": lambda: datetime_to_serial(datetime.datetime.now()),
    }

    def __init__(self, store, key):
        self.store = store
        self.key = key

    def sheet_of(self, text):
        if "!" not in text:
            return self.store.sheets[self.key], text
        name, ref = text.rsplit("!", 1)
        name = name.strip("'")
        for s in self.store.workbook["sheets"]:
            if s["name"].lower() == name.lower() and s["key"] in self.store.sheets:
                return self.store.sheets[s["key"]], ref
        raise NameError(name)

    def value(self, text):
        sheet, ref = self.sheet_of(text)
        at = parse_ref(ref.replace("$", ""))
        return sheet.value_at(*at)

    def values(self, text):
        sheet, ref = self.sheet_of(text)
        r0, c0, r1, c1 = parse_range(ref.replace("$", ""))
        return [sheet.value_at(r, c) for (r, c) in sorted(sheet.cells) if r0 <= r <= r1 and c0 <= c <= c1]

    def run(self, formula):
        tokens = tokenize(formula)
        expr = []
        for kind, value, _ in tokens:
            if kind == "string":
                expr.append(repr(value[1:-1].replace('""', '"')))
            elif kind == "range":
                expr.append(f"_r({value!r})")
            elif kind == "ref":
                expr.append(f"_v({value!r})")
            elif kind == "func":
                name = value.upper()
                if name not in self.FUNCTIONS:
                    return "#NAME?"
                expr.append(f"_f[{name!r}](")
            elif kind == "bool":
                expr.append(value.upper() == "TRUE" and "True" or "False")
            elif kind == "number":
                expr.append(value)
            elif kind == "name":
                return "#NAME?"
            else:
                expr.append({"^": "**", "=": "==", "<>": "!=", "&": "+", "%": "/100"}.get(value, value))
        source = " ".join(expr)
        try:
            result = eval(source, {"__builtins__": {}}, {"_r": self.values, "_v": lambda t: num_or(self.value(t)),
                                                          "_f": self.FUNCTIONS})
        except ZeroDivisionError:
            return "#DIV/0!"
        except NameError:
            return "#NAME?"
        except (TypeError, ValueError, AttributeError):
            return "#VALUE!"
        except SyntaxError:
            return "#VALUE!"
        return result


def flat(args):
    for a in args:
        if isinstance(a, list):
            yield from a
        else:
            yield a


def mean(values):
    if not values:
        raise ZeroDivisionError
    return sum(values) / len(values)


def num(x):
    if isinstance(x, bool):
        return int(x)
    if isinstance(x, (int, float)):
        return x
    if x in (None, ""):
        return 0
    return float(x)


def num_or(x):
    return 0 if x is None else x


# Runs of the layout -----------------------------------------------------------

def paint_runs(runs, first, last, change):
    """Apply `change(size, hidden, style) -> (size, hidden, style)` over [first, last]."""
    out = []
    covered = []
    for run in sorted(runs, key=lambda r: r[0]):
        a, b = run[0], run[1]
        if b < first or a > last:
            out.append(list(run))
            continue
        if a < first:
            out.append([a, first - 1] + list(run[2:]))
        if b > last:
            out.append([last + 1, b] + list(run[2:]))
        lo, hi = max(a, first), min(b, last)
        covered.append((lo, hi, run))
        out.append([lo, hi] + list(change(run[2], run[3], run[4])))
    at = first
    for lo, hi, _ in sorted(covered):
        if lo > at:
            out.append([at, lo - 1] + list(change(None, False, None)))
        at = hi + 1
    if at <= last:
        out.append([at, last] + list(change(None, False, None)))
    out = [r for r in out if not (r[2] is None and not r[3] and r[4] is None)]
    out.sort(key=lambda r: r[0])
    merged = []
    for r in out:
        if merged and merged[-1][1] + 1 == r[0] and merged[-1][2:] == r[2:]:
            merged[-1][1] = r[1]
        else:
            merged.append(r)
    return merged


def shift_runs(runs, at, count, limit, insert):
    out = []
    for run in runs:
        a, b = run[0], run[1]
        if insert:
            if a >= at:
                a, b = a + count, b + count
            elif b >= at:
                b = b + count
            if a < limit:
                out.append([a, min(b, limit - 1)] + list(run[2:]))
        else:
            end = at + count
            if b < at:
                out.append(list(run))
            elif a >= end:
                out.append([a - count, b - count] + list(run[2:]))
            else:
                na, nb = min(a, at), b - count if b >= end else at - 1
                if nb >= na:
                    out.append([na, nb] + list(run[2:]))
    return out


# Sheets -----------------------------------------------------------------------

class Sheet:
    def __init__(self, key, layout, cells):
        self.key = key
        self.layout = layout
        self.cells = {(item["r"], item["c"]): item for item in cells}
        self.reindex()

    def reindex(self):
        self.tiles = {}
        self.by_column = {}
        self.by_row = {}
        for (r, c), item in self.cells.items():
            self.tiles.setdefault((r // TILE_ROWS, c // TILE_COLUMNS), []).append(item)
            if item["text"] != "":
                self.by_column.setdefault(c, []).append(r)
                self.by_row.setdefault(r, []).append(c)
        for v in self.by_column.values():
            v.sort()
        for v in self.by_row.values():
            v.sort()
        self.merges = [parse_range(m) for m in self.layout.get("merges", [])]
        if self.cells:
            rows = [r for r, _ in self.cells]
            cols = [c for _, c in self.cells]
            self.layout["dimension"] = range_name((0, 0, max(rows), max(cols))) if min(rows) == 0 and min(cols) == 0 \
                else range_name((min(rows), min(cols), max(rows), max(cols)))
        else:
            self.layout["dimension"] = "A1"
        self.layout["cells"] = len(self.cells)

    def filled(self, r, c):
        item = self.cells.get((r, c))
        return bool(item and item["text"] != "")

    def value_at(self, r, c):
        item = self.cells.get((r, c))
        if not item or item["text"] == "" and item.get("value") is None:
            return None
        if "value" in item:
            return item["value"]
        kind = item["flags"] & 3
        if kind == 2:
            return item["text"] == "TRUE"
        if kind == 1:
            item["value"] = derive_number(item)
            return item["value"]
        return item["text"]

    def blank_style(self, r, c):
        for run in self.layout.get("rows", []):
            if run[0] <= r <= run[1] and run[4] is not None:
                return run[4]
        for run in self.layout.get("columns", []):
            if run[0] <= c <= run[1] and run[4] is not None:
                return run[4]
        return 0

    def tile(self, tr, tc):
        out = []
        for item in self.tiles.get((tr, tc), []):
            row = [item["r"] - tr * TILE_ROWS, item["c"] - tc * TILE_COLUMNS, item["text"], item["style"], item["flags"]]
            extra = [item.get("color"), item.get("fill"), item.get("shorter")]
            while extra and extra[-1] is None:
                extra.pop()
            out.append(row + extra)
        out.sort(key=lambda x: (x[0], x[1]))
        r0, c0 = tr * TILE_ROWS, tc * TILE_COLUMNS
        merges = [range_name(g) for g in self.merges
                  if g[0] < r0 + TILE_ROWS and g[2] >= r0 and g[1] < c0 + TILE_COLUMNS and g[3] >= c0]
        return {"cells": out, "merges": merges}

    def edge(self, r, c, direction):
        if direction in ("down", "up"):
            line = self.by_column.get(c, [])
            step = 1 if direction == "down" else -1
            limit = MAX_ROWS - 1 if step > 0 else 0
            if r == limit:
                return r, c
            if self.filled(r, c) and self.filled(r + step, c):
                n = r + step
                while n != limit and self.filled(n + step, c):
                    n += step
                return n, c
            if step > 0:
                i = bisect.bisect_right(line, r)
                return (line[i] if i < len(line) else limit), c
            i = bisect.bisect_left(line, r) - 1
            return (line[i] if i >= 0 else limit), c
        line = self.by_row.get(r, [])
        step = 1 if direction == "right" else -1
        limit = MAX_COLUMNS - 1 if step > 0 else 0
        if c == limit:
            return r, c
        if self.filled(r, c) and self.filled(r, c + step):
            n = c + step
            while n != limit and self.filled(r, n + step):
                n += step
            return r, n
        if step > 0:
            i = bisect.bisect_right(line, c)
            return r, (line[i] if i < len(line) else limit)
        i = bisect.bisect_left(line, c) - 1
        return r, (line[i] if i >= 0 else limit)

    def any_in_row(self, r, c0, c1):
        if r < 0 or r >= MAX_ROWS:
            return False
        line = self.by_row.get(r, [])
        i = bisect.bisect_left(line, max(0, c0))
        return i < len(line) and line[i] <= c1

    def any_in_column(self, c, r0, r1):
        if c < 0 or c >= MAX_COLUMNS:
            return False
        line = self.by_column.get(c, [])
        i = bisect.bisect_left(line, max(0, r0))
        return i < len(line) and line[i] <= r1

    def region(self, r, c):
        r0, c0, r1, c1 = r, c, r, c
        changed = True
        while changed:
            changed = False
            if self.any_in_row(r1 + 1, c0 - 1, c1 + 1):
                r1 += 1
                changed = True
            if self.any_in_row(r0 - 1, c0 - 1, c1 + 1):
                r0 -= 1
                changed = True
            if self.any_in_column(c1 + 1, r0 - 1, r1 + 1):
                c1 += 1
                changed = True
            if self.any_in_column(c0 - 1, r0 - 1, r1 + 1):
                c0 -= 1
                changed = True
        return max(r0, 0), max(c0, 0), min(r1, MAX_ROWS - 1), min(c1, MAX_COLUMNS - 1)

    def cell_doc(self, r, c):
        item = self.cells.get((r, c))
        merge = next((m for m in self.merges if m[0] <= r <= m[2] and m[1] <= c <= m[3]), None)
        kinds = {0: "text", 1: "number", 2: "boolean", 3: "error"}
        doc = {"ref": ref_name(r, c), "entry": None, "text": "", "style": self.blank_style(r, c), "kind": "blank",
               "error": None, "stale": False, "pivot": None, "array": None,
               "merge": range_name(merge) if merge else None}
        if not item:
            return doc
        flags = item["flags"]
        kind = "blank" if item["text"] == "" and not flags & 4 else kinds[flags & 3]
        doc.update({"entry": item.get("entry", item["text"]) or None, "text": item["text"], "style": item["style"],
                    "kind": kind, "error": item["text"] if flags & 3 == 3 else None, "stale": bool(flags & 8),
                    "pivot": "PivotTable1" if flags & 64 else None})
        return doc


def derive_number(item):
    entry = item.get("entry") or ""
    if entry and not entry.startswith("="):
        kind, value, _ = parse_entry(entry)
        if kind == "number":
            return value
    text = item["text"].strip().replace("$", "").replace(",", "")
    negative = text.startswith("(") and text.endswith(")")
    text = text.strip("()")
    try:
        value = float(text.rstrip("%")) / (100 if text.endswith("%") else 1)
    except ValueError:
        return 0.0
    return -value if negative else value


def empty_layout():
    return {"defaults": {"columnWidth": DEFAULT_WIDTH, "rowHeight": 15, "maxDigitWidth": 7},
            "columns": [], "rows": [], "merges": [], "frozen": {"rows": 0, "columns": 0}, "dimension": "A1",
            "cells": 0, "pivots": [], "arrays": [], "blocking": [], "tables": []}


# The store --------------------------------------------------------------------

class Store:
    def __init__(self, fixtures):
        self.workbook = json.loads((fixtures / "workbook.json").read_text())
        self.workbook["instance"] = uuid7()
        self.styles = json.loads((fixtures / "styles.json").read_text())
        self.functions = json.loads((fixtures / "functions.json").read_text())
        self.media = json.loads((fixtures / "media.json").read_text())
        self.sheets = {}
        for sheet in self.workbook["sheets"]:
            layout = fixtures / f"sheet-{sheet['key']}-layout.json"
            if layout.exists():
                self.sheets[sheet["key"]] = Sheet(sheet["key"], json.loads(layout.read_text()),
                                                  json.loads((fixtures / f"sheet-{sheet['key']}-cells.json").read_text()))
        start = self.workbook["revision"]
        self.base_rev = {key: start for key in self.sheets}
        self.layout_rev = {key: start for key in self.sheets}
        self.tile_rev = {}
        self.ring = []
        self.undo = []
        self.redo = []
        self.edit_latency = 0
        self.hold_changes = False

    # The workbook document, current.
    def document(self):
        wb = self.workbook
        wb["dirty"] = wb["revision"] != wb["saved"]
        wb["undo"] = {"label": self.undo[-1]["label"] if self.undo else None}
        wb["redo"] = {"label": self.redo[-1]["label"] if self.redo else None}
        wb["styles"] = len(self.styles)
        return wb

    def tile_etag(self, key, tr, tc):
        rev = max(self.base_rev.get(key, 0), self.tile_rev.get((key, tr, tc), 0))
        return f'"t{self.workbook["generation"]}.{key}.{tr}.{tc}.{rev}"'

    def layout_etag(self, key):
        return f'"l{self.workbook["generation"]}.{key}.{self.layout_rev.get(key, 0)}"'

    def sheet(self, key, path="$.edit.sheet"):
        if not isinstance(key, int) or key not in self.sheets:
            raise Refusal(400, "invalid_record", f"{path}: expected the key of a worksheet, got {json.dumps(key)}", path)
        return self.sheets[key]

    def name_of(self, key):
        return next(s["name"] for s in self.workbook["sheets"] if s["key"] == key)

    # Styles -------------------------------------------------------------------

    def intern(self, base, change):
        style = copy.deepcopy(self.styles[base] if base < len(self.styles) else self.styles[0])
        change(style)
        for i, known in enumerate(self.styles):
            if known == style:
                return i
        self.styles.append(style)
        self.styles_added = True
        return len(self.styles) - 1

    def code_of(self, style):
        return self.styles[style]["numberFormat"] if style < len(self.styles) else "General"

    # Cells --------------------------------------------------------------------

    def display(self, item):
        """Rewrite a cell's tile facts from its value and style."""
        for key in ("fill", "color", "shorter"):
            item.pop(key, None)
        value = item.get("value")
        code = self.code_of(item["style"])
        flags = item["flags"] & ~3
        if isinstance(value, bool):
            item["text"] = "TRUE" if value else "FALSE"
            flags |= 2
        elif isinstance(value, (int, float)):
            text, fill, color, shorter = render(float(value), code)
            item["text"] = text
            flags |= 1
            if fill:
                item["fill"] = fill
            if color:
                item["color"] = color
            if shorter:
                item["shorter"] = shorter
        elif isinstance(value, str) and value in ERRORS:
            item["text"] = value
            flags |= 3
        elif isinstance(value, str):
            text, fill = render_text(value, code)
            item["text"] = text
            if fill:
                item["fill"] = fill
        item["flags"] = flags

    def set_entry(self, sheet, r, c, text, location):
        item = sheet.cells.get((r, c))
        style = item["style"] if item else sheet.blank_style(r, c)
        kind, value, fmt = parse_entry(text)
        if kind == "blank":
            if style:
                sheet.cells[(r, c)] = {"r": r, "c": c, "text": "", "style": style, "flags": 16, "entry": ""}
            else:
                sheet.cells.pop((r, c), None)
            return
        if len(text) > 32767:
            raise Refusal(400, "invalid_record", f"{location}: expected at most 32767 characters in a cell, got {len(text)}", location)
        if kind == "formula":
            try:
                tokenize(value)
            except FormulaError as error:
                raise Refusal(400, "parse", f"{location}: {error} at position {error.position} of the formula", location)
        if fmt and self.code_of(style) == "General":
            style = self.intern(style, lambda s: s.__setitem__("numberFormat", fmt))
        new = {"r": r, "c": c, "text": "", "style": style, "flags": 4 if kind == "formula" else 0, "entry": text}
        if kind != "formula":
            new["value"] = value
        sheet.cells[(r, c)] = new
        if kind == "formula":
            new["value"] = 0.0
        self.display(new)

    def references(self, key, entry):
        """(sheet key, range) for every reference a formula names."""
        out = []
        names = {x["name"].lower(): x["key"] for x in self.workbook["sheets"]}
        for m in re.finditer(r"(?:('[^']+'|[A-Za-z_][\w.]*)!)?\$?([A-Za-z]{1,3})\$?(\d{1,7})"
                             r"(?::\$?([A-Za-z]{1,3})\$?(\d{1,7}))?(?![\w(])", entry):
            sheet = names.get(m.group(1).strip("'").lower()) if m.group(1) else key
            r0, c0 = int(m.group(3)) - 1, col_index(m.group(2))
            r1, c1 = (int(m.group(5)) - 1, col_index(m.group(4))) if m.group(4) else (r0, c0)
            out.append((sheet, (min(r0, r1), min(c0, c1), max(r0, r1), max(c0, c1))))
        return out

    def recalc(self):
        """Formulas reading a touched cell, and those reading them, a few levels deep."""
        count = 0
        frontier = list(self.touched)
        for _ in range(4):
            if not frontier:
                break
            moved = []
            for key, sheet in self.sheets.items():
                for item in sheet.cells.values():
                    if not item["flags"] & 4:
                        continue
                    here = (item["r"], item["c"], item["r"], item["c"])
                    if not any(k == key and intersects(g, here) for k, g in frontier) and not any(
                            (k, g2) for k, g in frontier for (k2, g2) in self.references(key, item["entry"])
                            if k2 == k and intersects(g, g2)):
                        continue
                    result = 0.0 if refers_to_itself(item) else Evaluator(self, key).run(item["entry"])
                    before = (item["text"], item.get("value"))
                    item["value"] = result
                    item["flags"] &= ~8
                    self.display(item)
                    if (item["text"], item.get("value")) != before:
                        count += 1
                        self.touched.append((key, here))
                        moved.append((key, here))
            frontier = moved
        return count

    # Ranges -------------------------------------------------------------------

    def ranges(self, values, path):
        if not isinstance(values, list) or not values:
            raise Refusal(400, "invalid_record", f"{path}: expected a list of ranges", path)
        out = []
        for i, text in enumerate(values):
            g = parse_range(text) if isinstance(text, str) else None
            if not g:
                raise Refusal(400, "invalid_record", f"{path}[{i}]: expected an A1 range, got {json.dumps(text)}", f"{path}[{i}]")
            out.append(g)
        return out

    def cells_in(self, sheet, g, create_limit=200000):
        """Every cell of a bounded range, or the present cells of a huge one."""
        if area(g) <= create_limit:
            for r in range(g[0], g[2] + 1):
                for c in range(g[1], g[3] + 1):
                    yield r, c
        else:
            for (r, c) in list(sheet.cells):
                if g[0] <= r <= g[2] and g[1] <= c <= g[3]:
                    yield r, c

    # Ops ------------------------------------------------------------------------

    def targets(self, edit):
        op = edit.get("op")
        key = edit.get("sheet")
        if op == "setEntries":
            return [(key, parse_range(e.get("ref", "A1")) or (0, 0, 0, 0)) for e in edit.get("entries", [])]
        if op in ("fillEntry", "clear", "setStyle"):
            return [(key, parse_range(t) or (0, 0, 0, 0)) for t in edit.get("ranges", [])]
        if op in ("merge", "unmerge", "sort"):
            return [(key, parse_range(edit.get("range", "A1")) or (0, 0, 0, 0))]
        if op == "fill":
            return [(key, parse_range(edit.get("target", "A1")) or (0, 0, 0, 0))]
        if op == "batch":
            return [t for e in edit.get("edits", []) for t in self.targets(e)]
        if op == "pasteText":
            at = parse_ref(edit.get("ref", "A1") or "A1") or (0, 0)
            return [(key, (at[0], at[1], at[0], at[1]))]
        if op == "paste":
            source, target = edit.get("from") or {}, edit.get("to") or {}
            g = parse_range(source.get("range", "A1") or "A1") or (0, 0, 0, 0)
            at = parse_ref(target.get("ref", "A1") or "A1") or (0, 0)
            out = [(target.get("sheet"), (at[0], at[1], at[0] + g[2] - g[0], at[1] + g[3] - g[1]))]
            if edit.get("cut"):
                out.append((source.get("sheet"), g))
            return out
        return [(key, None)]

    def stale(self, base, edit):
        if not isinstance(base, int):
            raise Refusal(400, "invalid_record", "$.base: expected the revision the edit was made against", "$.base")
        targets = self.targets(edit)
        for change in self.ring:
            if change["revision"] <= base:
                continue
            if change["structural"] or change["sheets"]:
                return True
            for k, g in change["touched"]:
                for tk, tg in targets:
                    if k == tk and (tg is None or g is None or intersects(g, tg)):
                        return True
        return False

    def apply_one(self, edit, path="$.edit"):
        if not isinstance(edit, dict):
            raise Refusal(400, "invalid_record", f"{path}: expected an object")
        op = edit.get("op")
        wb = self.workbook
        if op == "batch":
            for i, inner in enumerate(edit.get("edits", [])):
                self.apply_one(inner, f"{path}.edits[{i}]")
            return
        if op in ("addSheet", "renameSheet", "removeSheet", "moveSheet", "sheetState"):
            return self.sheet_op(op, edit, path)
        if op == "replace":
            return self.replace(edit, path)
        if op == "paste":
            # `paste` names its sheets in `from` and `to`.
            return self.paste(edit, path)
        sheet = self.sheet(edit.get("sheet"), path + ".sheet")
        key = sheet.key
        name = self.name_of(key)
        if op == "setEntries":
            entries = edit.get("entries")
            if not isinstance(entries, list) or not entries:
                raise Refusal(400, "invalid_record", f"{path}.entries: expected a list of entries")
            for i, entry in enumerate(entries):
                at = parse_ref(entry.get("ref", "")) if isinstance(entry, dict) else None
                text = entry.get("text") if isinstance(entry, dict) else None
                if not at or not isinstance(text, str):
                    raise Refusal(400, "invalid_record", f"{path}.entries[{i}]: expected {{ref, text}}", f"{path}.entries[{i}]")
                self.set_entry(sheet, at[0], at[1], text, f"{name}!{ref_name(*at)}")
                self.touched.append((key, (at[0], at[1], at[0], at[1])))
            return
        if op == "fillEntry":
            ranges = self.ranges(edit.get("ranges"), path + ".ranges")
            text = edit.get("text")
            if not isinstance(text, str):
                raise Refusal(400, "invalid_record", f"{path}.text: expected text")
            # Addendum 6: the text is entered at `ref`, the active cell, and
            # every other cell takes the same formula shape from there.
            host = parse_ref(edit.get("ref", "") or "") if "ref" in edit else ranges[0][:2]
            if not host:
                raise Refusal(400, "invalid_record", f"{path}.ref: expected the active cell's A1 reference", path + ".ref")
            for g in ranges:
                if area(g) > 200000:
                    raise Refusal(413, "too_large", f"{path}.ranges: fillEntry takes at most 200000 cells")
                for r, c in self.cells_in(sheet, g):
                    entry = shift_formula(text, r - host[0], c - host[1]) if text.startswith("=") else text
                    self.set_entry(sheet, r, c, entry, f"{name}!{ref_name(r, c)}")
                self.touched.append((key, g))
            return
        if op == "clear":
            ranges = self.ranges(edit.get("ranges"), path + ".ranges")
            what = edit.get("what", "all")
            for g in ranges:
                for r, c in list(self.cells_in(sheet, g, 0)):
                    item = sheet.cells.get((r, c))
                    if not item:
                        continue
                    if what == "all":
                        del sheet.cells[(r, c)]
                    elif what == "contents":
                        if item["style"]:
                            sheet.cells[(r, c)] = {"r": r, "c": c, "text": "", "style": item["style"], "flags": 16, "entry": ""}
                        else:
                            del sheet.cells[(r, c)]
                    elif what == "formats":
                        if item["text"] == "" and not item["flags"] & 4:
                            del sheet.cells[(r, c)]
                        else:
                            item["style"] = 0
                            if "value" not in item:
                                item["value"] = sheet.value_at(r, c)
                            self.display(item)
                    else:
                        raise Refusal(400, "invalid_record", f"{path}.what: expected all, contents or formats")
                self.touched.append((key, g))
            return
        if op == "setStyle":
            ranges = self.ranges(edit.get("ranges"), path + ".ranges")
            patch = edit.get("patch")
            if not isinstance(patch, dict):
                raise Refusal(400, "invalid_record", f"{path}.patch: expected an object")
            if "decimals" in patch and "numberFormat" in patch:
                raise Refusal(400, "invalid_record", f"{path}.patch: expected decimals or numberFormat, not both", path + ".patch")
            if "decimals" in patch and (not isinstance(patch["decimals"], int) or not -15 <= patch["decimals"] <= 15):
                raise Refusal(400, "invalid_record", f"{path}.patch.decimals: expected an integer from -15 to 15, "
                                                     f"got {json.dumps(patch['decimals'])}", path + ".patch.decimals")
            for g in ranges:
                derived = {}
                for r, c in self.cells_in(sheet, g):
                    item = sheet.cells.get((r, c))
                    base = item["style"] if item else sheet.blank_style(r, c)
                    borders = patch.get("borders")
                    edges = self.edges_for(borders, g, r, c) if borders else None
                    token = (base, json.dumps(edges, sort_keys=True))
                    if token not in derived:
                        value = sheet.value_at(r, c) if item else None
                        derived[token] = self.intern(base, lambda s: self.patch_style(s, patch, edges, value, path))
                    style = derived[token]
                    if item:
                        if item["style"] != style:
                            if "value" not in item and item["text"] != "":
                                item["value"] = sheet.value_at(r, c)
                            item["style"] = style
                            if item["text"] != "" or item["flags"] & 4:
                                self.display(item)
                    elif style:
                        sheet.cells[(r, c)] = {"r": r, "c": c, "text": "", "style": style, "flags": 16, "entry": ""}
                self.touched.append((key, g))
            return
        if op in ("insertRows", "removeRows", "insertColumns", "removeColumns"):
            rows = "Rows" in op
            insert = op.startswith("insert")
            at = edit.get("at" if insert else "start")
            count = edit.get("count")
            limit = MAX_ROWS if rows else MAX_COLUMNS
            if not isinstance(at, int) or not isinstance(count, int) or count < 1 or at < 0 or at >= limit:
                raise Refusal(400, "invalid_record", f"{path}: expected {'at' if insert else 'start'} and count within the sheet")
            if insert:
                lost = [p for p in sheet.cells if (p[0] if rows else p[1]) >= limit - count]
                if lost:
                    raise Refusal(400, "invalid_record", f"{name}!{ref_name(*lost[0])}: inserting would push cells off the sheet",
                                  f"{name}!{ref_name(*lost[0])}")
            moved = {}
            for (r, c), item in sheet.cells.items():
                index = r if rows else c
                if insert:
                    if index >= at:
                        index += count
                else:
                    if at <= index < at + count:
                        continue
                    if index >= at + count:
                        index -= count
                nr, nc = (index, c) if rows else (r, index)
                item["r"], item["c"] = nr, nc
                moved[(nr, nc)] = item
            sheet.cells = moved
            axis = "rows" if rows else "columns"
            sheet.layout[axis] = shift_runs(sheet.layout.get(axis, []), at, count, limit, insert)
            merges = []
            for g in sheet.merges:
                lo, hi = (g[0], g[2]) if rows else (g[1], g[3])
                if insert:
                    if lo >= at:
                        lo, hi = lo + count, hi + count
                    elif hi >= at:
                        hi += count
                else:
                    end = at + count
                    if hi < at:
                        pass
                    elif lo >= end:
                        lo, hi = lo - count, hi - count
                    elif lo >= at and hi < end:
                        continue
                    else:
                        lo, hi = min(lo, at), hi - count if hi >= end else at - 1
                ng = (lo, g[1], hi, g[3]) if rows else (g[0], lo, g[2], hi)
                if area(ng) > 1:
                    merges.append(range_name(ng))
            sheet.layout["merges"] = merges
            for item in sheet.cells.values():
                if item["flags"] & 4:
                    item["entry"] = shift_references(item["entry"], rows, at, count, insert)
            self.structural.add(key)
            self.touched.append((key, (0, 0, MAX_ROWS - 1, MAX_COLUMNS - 1)))
            return
        if op in ("rowHeight", "columnWidth", "hideRows", "hideColumns"):
            rows = op in ("rowHeight", "hideRows")
            start, count = edit.get("start"), edit.get("count")
            limit = MAX_ROWS if rows else MAX_COLUMNS
            if not isinstance(start, int) or not isinstance(count, int) or count < 1 or start < 0 or start + count > limit:
                raise Refusal(400, "invalid_record", f"{path}: expected start and count within the sheet")
            axis = "rows" if rows else "columns"
            if op in ("rowHeight", "columnWidth"):
                size = edit.get("size")
                if size is not None and (not isinstance(size, (int, float)) or size < 0 or size > (409 if rows else 255)):
                    raise Refusal(400, "invalid_record", f"{path}.size: expected a size from 0 to {409 if rows else 255}, or null")
                change = lambda s, h, st: (size if size else None, size == 0 or (h and size is None), st)
            else:
                hidden = edit.get("hidden")
                if not isinstance(hidden, bool):
                    raise Refusal(400, "invalid_record", f"{path}.hidden: expected true or false")
                change = lambda s, h, st: (s, hidden, st)
            sheet.layout[axis] = paint_runs(sheet.layout.get(axis, []), start, start + count - 1, change)
            return
        if op == "merge":
            g = parse_range(edit.get("range", "") or "")
            if not g:
                raise Refusal(400, "invalid_record", f"{path}.range: expected an A1 range")
            parts = [(r, g[1], r, g[3]) for r in range(g[0], g[2] + 1)] if edit.get("across") else [g]
            for part in parts:
                if area(part) < 2:
                    continue
                for other in sheet.merges:
                    if intersects(other, part):
                        raise Refusal(400, "invalid_record",
                                      f"{name}!{range_name(part)}: overlaps the merged range {range_name(other)}")
                for (r, c) in list(sheet.cells):
                    if part[0] <= r <= part[2] and part[1] <= c <= part[3] and (r, c) != part[:2]:
                        item = sheet.cells[(r, c)]
                        if item["style"]:
                            sheet.cells[(r, c)] = {"r": r, "c": c, "text": "", "style": item["style"], "flags": 16, "entry": ""}
                        else:
                            del sheet.cells[(r, c)]
                sheet.layout.setdefault("merges", []).append(range_name(part))
                sheet.merges.append(part)
                if edit.get("center"):
                    item = sheet.cells.get(part[:2])
                    base = item["style"] if item else sheet.blank_style(*part[:2])
                    style = self.intern(base, lambda s: s["alignment"].__setitem__("horizontal", "center"))
                    if item:
                        item["style"] = style
                        if "value" not in item and item["text"]:
                            item["value"] = sheet.value_at(*part[:2])
                        self.display(item)
                    else:
                        sheet.cells[part[:2]] = {"r": part[0], "c": part[1], "text": "", "style": style, "flags": 16, "entry": ""}
                self.touched.append((key, part))
            return
        if op == "unmerge":
            g = parse_range(edit.get("range", "") or "")
            if not g:
                raise Refusal(400, "invalid_record", f"{path}.range: expected an A1 range")
            kept = [m for m in sheet.merges if not (intersects(m, g))]
            sheet.layout["merges"] = [range_name(m) for m in kept]
            self.touched.append((key, g))
            return
        if op == "freeze":
            rows, columns = edit.get("rows", 0), edit.get("columns", 0)
            sheet.layout["frozen"] = {"rows": rows, "columns": columns}
            return
        if op == "fill":
            source = parse_range(edit.get("source", "") or "")
            target = parse_range(edit.get("target", "") or "")
            mode = edit.get("mode", "copy")
            if not source or not target:
                raise Refusal(400, "invalid_record", f"{path}: expected source and target ranges")
            self.fill(sheet, source, target, mode, path)
            self.touched.append((key, target))
            return
        if op == "sort":
            g = parse_range(edit.get("range", "") or "")
            keys = edit.get("keys") or []
            if not g or not keys:
                raise Refusal(400, "invalid_record", f"{path}: expected a range and at least one key")
            self.sort(sheet, g, keys, bool(edit.get("header")))
            self.touched.append((key, g))
            return
        if op == "pasteText":
            at = parse_ref(edit.get("ref", "") or "")
            text = edit.get("text")
            if not at or not isinstance(text, str):
                raise Refusal(400, "invalid_record", f"{path}: expected ref and text")
            lines = split_tsv(text)
            width = 0
            for i, cells in enumerate(lines):
                width = max(width, len(cells))
                for j, value in enumerate(cells):
                    self.set_entry(sheet, at[0] + i, at[1] + j, value, f"{name}!{ref_name(at[0] + i, at[1] + j)}")
            self.touched.append((key, (at[0], at[1], at[0] + len(lines) - 1, at[1] + max(width, 1) - 1)))
            return
        raise Refusal(400, "invalid_record", f"{path}.op: expected one of §8.4's ops, got {json.dumps(op)}", path + ".op")

    def edges_for(self, borders, g, r, c):
        preset = borders.get("preset")
        style = borders.get("style") or ("thick" if preset == "thickOutside" else "thin")
        color = borders.get("color") or "#000000"
        edge = {"style": style, "color": color}
        sides = {}
        if preset == "all":
            sides = {"left": edge, "right": edge, "top": edge, "bottom": edge}
        elif preset in ("outside", "thickOutside"):
            if r == g[0]:
                sides["top"] = edge
            if r == g[2]:
                sides["bottom"] = edge
            if c == g[1]:
                sides["left"] = edge
            if c == g[3]:
                sides["right"] = edge
        elif preset == "bottom" and r == g[2]:
            sides["bottom"] = edge
        elif preset == "top" and r == g[0]:
            sides["top"] = edge
        elif preset == "left" and c == g[1]:
            sides["left"] = edge
        elif preset == "right" and c == g[3]:
            sides["right"] = edge
        elif preset == "none":
            return {"none": True}
        elif preset not in ("bottom", "top", "left", "right"):
            raise Refusal(400, "invalid_record", f"$.edit.patch.borders.preset: expected bottom, top, left, right, all, "
                                                 f"outside, thickOutside or none, got {json.dumps(preset)}")
        return sides

    def patch_style(self, style, patch, edges, value, path):
        font, alignment = style["font"], style["alignment"]
        for wire, field in (("fontName", "name"), ("fontSize", "size"), ("bold", "bold"), ("italic", "italic"),
                            ("underline", "underline"), ("strike", "strike")):
            if wire in patch:
                font[field] = patch[wire]
        if "fontColor" in patch:
            font["color"] = patch["fontColor"] or "#000000"
        if "fill" in patch:
            style["fill"] = {"pattern": "solid", "color": patch["fill"]} if patch["fill"] else {"pattern": "none"}
        for key in ("horizontal", "vertical", "wrap", "indent"):
            if key in patch:
                alignment[key] = patch[key] if patch[key] is not None else {"horizontal": "general", "vertical": "bottom",
                                                                            "wrap": False, "indent": 0}[key]
        if "numberFormat" in patch:
            style["numberFormat"] = patch["numberFormat"] or "General"
        if "decimals" in patch:
            style["numberFormat"] = with_decimals(style["numberFormat"], int(patch["decimals"]),
                                                  value if isinstance(value, (int, float)) else None)
        if edges is not None:
            if edges.get("none"):
                style["border"] = {}
            else:
                style["border"] = {**style.get("border", {}), **edges}

    def fill(self, sheet, source, target, mode, path):
        down = target[1] == source[1] and target[3] == source[3] and target[0] == source[0] and target[2] > source[2]
        up = target[1] == source[1] and target[3] == source[3] and target[2] == source[2] and target[0] < source[0]
        right = target[0] == source[0] and target[2] == source[2] and target[1] == source[1] and target[3] > source[3]
        left = target[0] == source[0] and target[2] == source[2] and target[3] == source[3] and target[1] < source[1]
        if not (down or up or right or left):
            raise Refusal(400, "invalid_record", f"{path}.target: expected a range extending the source in one direction")
        lines = range(source[1], source[3] + 1) if down or up else range(source[0], source[2] + 1)
        for line in lines:
            if down or up:
                src = [(r, line) for r in range(source[0], source[2] + 1)]
                dst = [(r, line) for r in (range(source[2] + 1, target[2] + 1) if down else range(source[0] - 1, target[0] - 1, -1))]
            else:
                src = [(line, c) for c in range(source[1], source[3] + 1)]
                dst = [(line, c) for c in (range(source[3] + 1, target[3] + 1) if right else range(source[1] - 1, target[1] - 1, -1))]
            if up or left:
                src = list(reversed(src))
            items = [sheet.cells.get(p) for p in src]
            values = [sheet.value_at(*p) for p in src]
            numeric = mode == "series" and all(isinstance(v, (int, float)) and not isinstance(v, bool) for v in values) \
                and all(i and not i["flags"] & 4 for i in items)
            step = (values[-1] - values[0]) / (len(values) - 1) if numeric and len(values) > 1 else 0
            for n, (r, c) in enumerate(dst):
                i = n % len(src)
                item = items[i]
                if numeric and len(values) > 1:
                    value = values[-1] + step * (n + 1)
                    self.set_entry(sheet, r, c, general(value)[0], ref_name(r, c))
                    sheet.cells[(r, c)]["style"] = item["style"]
                    sheet.cells[(r, c)]["value"] = value
                    self.display(sheet.cells[(r, c)])
                    continue
                if not item:
                    sheet.cells.pop((r, c), None)
                    continue
                new = copy.deepcopy(item)
                new["r"], new["c"] = r, c
                if new["flags"] & 4:
                    new["entry"] = shift_formula(new["entry"], r - src[i][0], c - src[i][1])
                sheet.cells[(r, c)] = new

    def sort(self, sheet, g, keys, header):
        r0 = g[0] + (1 if header else 0)
        rows = []
        for r in range(r0, g[2] + 1):
            rows.append({c: sheet.cells.pop((r, c)) for c in range(g[1], g[3] + 1) if (r, c) in sheet.cells})

        def rank(row, column, descending):
            item = row.get(column)
            value = None
            if item and item["text"] != "":
                value = item.get("value")
                if value is None:
                    kind = item["flags"] & 3
                    value = derive_number(item) if kind == 1 else (item["text"] == "TRUE" if kind == 2 else item["text"])
            if value is None:
                return (1, 0, 0)
            if isinstance(value, bool):
                order = (2, int(value))
            elif isinstance(value, (int, float)):
                order = (0, value)
            elif value in ERRORS:
                order = (3, value)
            else:
                order = (1, value.casefold())
            return (0, order, 0)

        for spec in reversed(keys):
            column = col_index(spec.get("column", "A"))
            descending = bool(spec.get("descending"))
            filled = [row for row in rows if rank(row, column, descending)[0] == 0]
            blank = [row for row in rows if rank(row, column, descending)[0] == 1]
            filled.sort(key=lambda row: rank(row, column, descending)[1], reverse=descending)
            rows = filled + blank
        for i, row in enumerate(rows):
            r = r0 + i
            for c, item in row.items():
                dr = r - item["r"]
                item["r"] = r
                if item["flags"] & 4:
                    item["entry"] = shift_formula(item["entry"], dr, 0)
                sheet.cells[(r, c)] = item

    def paste(self, edit, path):
        source = edit.get("from") or {}
        target = edit.get("to") or {}
        what = edit.get("what", "all")
        src_sheet = self.sheet(source.get("sheet"), path + ".from.sheet")
        dst_sheet = self.sheet(target.get("sheet"), path + ".to.sheet")
        g = parse_range(source.get("range", "") or "")
        at = parse_ref(target.get("ref", "") or "")
        if not g or not at:
            raise Refusal(400, "invalid_record", f"{path}: expected from.range and to.ref")
        items = {(r - g[0], c - g[1]): copy.deepcopy(i) for (r, c), i in src_sheet.cells.items()
                 if g[0] <= r <= g[2] and g[1] <= c <= g[3]}
        if edit.get("cut"):
            for (dr, dc) in items:
                src_sheet.cells.pop((g[0] + dr, g[1] + dc), None)
            self.touched.append((src_sheet.key, g))
        out = (at[0], at[1], at[0] + g[2] - g[0], at[1] + g[3] - g[1])
        for r in range(out[0], out[2] + 1):
            for c in range(out[1], out[3] + 1):
                item = items.get((r - at[0], c - at[1]))
                old = dst_sheet.cells.get((r, c))
                if what == "formats":
                    if item:
                        if old:
                            old["style"] = item["style"]
                            if "value" not in old and old["text"]:
                                old["value"] = dst_sheet.value_at(r, c)
                            self.display(old)
                        else:
                            dst_sheet.cells[(r, c)] = {"r": r, "c": c, "text": "", "style": item["style"], "flags": 16, "entry": ""}
                    continue
                if not item:
                    if what == "all":
                        dst_sheet.cells.pop((r, c), None)
                    continue
                new = copy.deepcopy(item)
                new["r"], new["c"] = r, c
                if new["flags"] & 4:
                    if what == "values":
                        new["flags"] &= ~4
                        new["entry"] = new["text"]
                    elif not edit.get("cut"):
                        new["entry"] = shift_formula(new["entry"], r - (g[0] + r - at[0]), c - (g[1] + c - at[1]))
                if what in ("values", "formulas") and old:
                    new["style"] = old["style"]
                    self.display(new)
                elif what in ("values", "formulas"):
                    new["style"] = 0
                    self.display(new)
                dst_sheet.cells[(r, c)] = new
        self.touched.append((dst_sheet.key, out))

    # Find and replace (§4.3, addendum 10) ------------------------------------------

    def search_keys(self, start, scope):
        """The worksheets a search walks from `start`: it alone, or every visible one from it on, wrapping."""
        if scope == "sheet":
            return [start]
        keys = [s["key"] for s in self.workbook["sheets"] if s["key"] in self.sheets and s["state"] == "visible"]
        if start not in keys:
            return keys
        at = keys.index(start)
        return keys[at:] + keys[:at]

    @staticmethod
    def searched(item, within):
        if item["text"] == "" and not item.get("entry"):
            return None
        return (item.get("entry") or item["text"]) if within == "formulas" else item["text"]

    def find(self, text, scope, start, origin, within, match_case, entire):
        pattern = wildcard(text, match_case, entire)
        keys = self.search_keys(start, scope)
        after = [(start, lambda rc: origin is None or rc > origin)] + [(k, lambda rc: True) for k in keys[1:]]
        if origin is not None:
            after.append((start, lambda rc: rc <= origin))
        for key, wanted in after:
            for rc, item in sorted(self.sheets[key].cells.items()):
                source = self.searched(item, within)
                if source is not None and wanted(rc) and pattern.search(source):
                    return {"sheet": key, "ref": ref_name(*rc)}
        return None

    def replace(self, edit, path):
        find = edit.get("text")
        if not isinstance(find, str) or not find:
            raise Refusal(400, "invalid_record", f"{path}.text: expected the text to find")
        replacement = edit.get("replacement", "")
        if not isinstance(replacement, str):
            raise Refusal(400, "invalid_record", f"{path}.replacement: expected text")
        scope = edit.get("scope", "sheet")
        within = edit.get("in", "formulas")
        if scope not in ("sheet", "workbook") or within not in ("values", "formulas"):
            raise Refusal(400, "invalid_record", f"{path}: expected scope sheet|workbook and in values|formulas")
        start = self.sheet(edit.get("sheet"), path + ".sheet")
        entire = bool(edit.get("entireCell"))
        pattern = wildcard(find, bool(edit.get("matchCase")), entire)
        # Every replacement is checked before any cell changes.
        planned = []
        for key in self.search_keys(start.key, scope):
            sheet = self.sheets[key]
            for (r, c), item in sorted(sheet.cells.items()):
                source = self.searched(item, "formulas")
                if source is None or not pattern.search(source):
                    continue
                new = replacement if entire else pattern.sub(lambda m: replacement, source)
                if new.startswith("="):
                    try:
                        tokenize(new[1:])
                    except FormulaError as error:
                        location = f"{self.name_of(key)}!{ref_name(r, c)}"
                        raise Refusal(400, "parse", f"{location}: the replacement makes {new!r}, which is not a formula: "
                                                    f"{error}", location)
                planned.append((key, r, c, new))
        for key, r, c, new in planned:
            self.set_entry(self.sheets[key], r, c, new, f"{self.name_of(key)}!{ref_name(r, c)}")
            self.touched.append((key, (r, c, r, c)))
        self.result = {"replaced": len(planned)}

    def format_preview(self, code, key, at):
        """addendum 9: the code rendered against the cell's value, 1234.5 on a blank."""
        sheet = self.sheets[key]
        value = sheet.value_at(*at)
        if value is None or value == "":
            text, fill, color, _ = render(1234.5, code)
        elif isinstance(value, bool):
            text, fill, color = ("TRUE" if value else "FALSE"), None, None
        elif isinstance(value, (int, float)):
            text, fill, color, _ = render(float(value), code)
        elif value in ERRORS:
            text, fill, color = value, None, None
        else:
            text, fill = render_text(value, code)
            color = None
        return {"text": text, "color": color, "fill": fill}

    def sheet_op(self, op, edit, path):
        wb = self.workbook
        sheets = wb["sheets"]
        if op == "addSheet":
            name = edit.get("name")
            keys = [s["key"] for s in sheets]
            key = max(keys) + 1 if keys else 1
            if not name:
                n = 1
                while any(s["name"].lower() == f"sheet{n}" for s in sheets):
                    n += 1
                name = f"Sheet{n}"
            self.check_name(name, None, path + ".name")
            at = edit.get("at")
            at = len(sheets) if at is None else max(0, min(int(at), len(sheets)))
            sheets.insert(at, {"key": key, "name": name, "kind": "worksheet", "state": "visible"})
            self.sheets[key] = Sheet(key, empty_layout(), [])
            self.base_rev[key] = self.layout_rev[key] = 0
            self.result = {"sheet": key}
            self.sheet_list = True
            return
        key = edit.get("sheet")
        entry = next((s for s in sheets if s["key"] == key), None)
        if not entry:
            raise Refusal(400, "invalid_record", f"{path}.sheet: expected a sheet key, got {json.dumps(key)}")
        if op == "renameSheet":
            name = edit.get("name")
            self.check_name(name, key, path + ".name")
            old = entry["name"]
            entry["name"] = name
            for sheet in self.sheets.values():
                for item in sheet.cells.values():
                    if item["flags"] & 4:
                        item["entry"] = re.sub(r"(?<![\w'])'?" + re.escape(old) + r"'?!", lambda m: quote_sheet(name) + "!",
                                               item["entry"])
        elif op == "removeSheet":
            visible = [s for s in sheets if s["state"] == "visible"]
            if entry["state"] == "visible" and len(visible) == 1:
                raise Refusal(400, "invalid_record", f"{entry['name']}: a workbook keeps at least one visible sheet")
            sheets.remove(entry)
            self.sheets.pop(key, None)
            if wb.get("activeSheet") == key:
                wb["activeSheet"] = next(s["key"] for s in sheets if s["state"] == "visible")
        elif op == "moveSheet":
            to = edit.get("to")
            if not isinstance(to, int):
                raise Refusal(400, "invalid_record", f"{path}.to: expected a position")
            sheets.remove(entry)
            sheets.insert(max(0, min(to, len(sheets))), entry)
        elif op == "sheetState":
            state = edit.get("state")
            if state not in ("visible", "hidden", "veryHidden"):
                raise Refusal(400, "invalid_record", f"{path}.state: expected visible, hidden or veryHidden")
            visible = [s for s in sheets if s["state"] == "visible"]
            if state != "visible" and entry["state"] == "visible" and len(visible) == 1:
                raise Refusal(400, "invalid_record", f"{entry['name']}: a workbook keeps at least one visible sheet")
            entry["state"] = state
        self.sheet_list = True

    def check_name(self, name, key, path):
        if not isinstance(name, str) or not name.strip() or len(name) > 31 or re.search(r"[\[\]:*?/\\]", name) \
                or name.startswith("'") or name.endswith("'"):
            raise Refusal(400, "invalid_record", f"{path}: expected a sheet name of 1 to 31 characters without [ ] : * ? / \\")
        if any(s["name"].lower() == name.lower() and s["key"] != key for s in self.workbook["sheets"]):
            raise Refusal(400, "invalid_record", f"{path}: a sheet named '{name}' already exists")

    # The door ---------------------------------------------------------------------

    def snapshot(self):
        return copy.deepcopy({"sheets": {k: (s.layout, s.cells) for k, s in self.sheets.items()},
                              "list": self.workbook["sheets"]})

    def restore(self, snap):
        snap = copy.deepcopy(snap)
        self.workbook["sheets"] = snap["list"]
        self.sheets = {k: Sheet.__new__(Sheet) for k in snap["sheets"]}
        for k, (layout, cells) in snap["sheets"].items():
            sheet = self.sheets[k]
            sheet.key, sheet.layout, sheet.cells = k, layout, cells
            sheet.reindex()
            self.base_rev.setdefault(k, 0)
            self.layout_rev.setdefault(k, 0)

    def label(self, edit):
        op = edit.get("op")
        if op == "setEntries" and len(edit.get("entries", [])) == 1:
            e = edit["entries"][0]
            text = e.get("text", "")
            return f"Typing '{text[:40]}' in {e.get('ref')}" if text else f"Clear {e.get('ref')}"
        return {"setEntries": "Typing", "fillEntry": "Typing", "clear": "Clear", "setStyle": "Format Cells",
                "insertRows": "Insert Rows", "removeRows": "Delete Rows", "insertColumns": "Insert Columns",
                "removeColumns": "Delete Columns", "rowHeight": "Row Height", "columnWidth": "Column Width",
                "hideRows": "Hide Rows", "hideColumns": "Hide Columns", "merge": "Merge Cells", "unmerge": "Unmerge Cells",
                "freeze": "Freeze Panes", "fill": "Fill", "sort": "Sort", "paste": "Paste", "pasteText": "Paste",
                "addSheet": "Insert Sheet", "renameSheet": "Rename Sheet", "removeSheet": "Delete Sheet",
                "moveSheet": "Move Sheet", "sheetState": "Hide Sheet", "replace": "Replace", "batch": "Batch"}.get(op, str(op))

    def begin(self):
        self.touched = []
        self.structural = set()
        self.sheet_list = False
        self.styles_added = False
        self.result = {}

    def commit(self, before, layouts_before):
        """Bump the revision, the touched tiles and layouts; record the change."""
        wb = self.workbook
        wb["revision"] += 1
        rev = wb["revision"]
        for sheet in self.sheets.values():
            sheet.reindex()
        changed = []
        for key, g in self.touched:
            if key not in self.sheets:
                continue
            tiles = (g[2] // TILE_ROWS - g[0] // TILE_ROWS + 1) * (g[3] // TILE_COLUMNS - g[1] // TILE_COLUMNS + 1)
            if tiles > 4096 or key in self.structural:
                self.base_rev[key] = rev
            else:
                for tr in range(g[0] // TILE_ROWS, g[2] // TILE_ROWS + 1):
                    for tc in range(g[1] // TILE_COLUMNS, g[3] // TILE_COLUMNS + 1):
                        self.tile_rev[(key, tr, tc)] = rev
            item = {"sheet": key, "range": range_name(g)}
            if item not in changed:
                changed.append(item)
        for key, sheet in self.sheets.items():
            if json.dumps(sheet.layout.get("rows")) + json.dumps(sheet.layout.get("columns")) + \
                    json.dumps(sheet.layout.get("merges")) + json.dumps(sheet.layout.get("frozen")) != layouts_before.get(key):
                self.layout_rev[key] = rev
        structural = bool(self.structural)
        change = {"revision": rev, "changed": changed, "structural": structural, "sheets": self.sheet_list,
                  "styles": self.styles_added,
                  "touched": [(k, g) for k, g in self.touched] + ([(k, None) for k in self.structural])}
        self.ring.append(change)
        del self.ring[:-RING]
        calc = self.workbook.get("calc") or {}
        return {"revision": rev, "changed": changed, "structural": structural, "sheets": self.sheet_list,
                "styles": self.styles_added, "undoable": True,
                "calc": {"evaluated": self.evaluated, "uncomputed": calc.get("uncomputed", 0),
                         "circular": calc.get("circular", [])}, "result": self.result}

    def layouts(self):
        return {key: json.dumps(s.layout.get("rows")) + json.dumps(s.layout.get("columns")) +
                json.dumps(s.layout.get("merges")) + json.dumps(s.layout.get("frozen")) for key, s in self.sheets.items()}

    def edit(self, body):
        if self.workbook.get("readOnly"):
            raise Refusal(403, "read_only", "the workbook is open read-only")
        if not isinstance(body, dict) or "edit" not in body:
            raise Refusal(400, "invalid_record", "$: expected {base, edit}")
        edit = body["edit"]
        if self.stale(body.get("base"), edit if isinstance(edit, dict) else {}):
            raise Refusal(409, "stale_base", f"the workbook changed after revision {body.get('base')} where this edit "
                                              f"was made; read the change and try again")
        before = self.snapshot()
        layouts = self.layouts()
        self.begin()
        try:
            self.apply_one(edit)
            self.evaluated = self.recalc()
        except Exception:
            self.restore(before)
            raise
        answer = self.commit(before, layouts)
        self.undo.append({"before": before, "after": self.snapshot(), "label": self.label(edit),
                          "touched": list(self.touched), "structural": set(self.structural), "sheets": self.sheet_list})
        del self.undo[:-100]
        self.redo.clear()
        return answer

    def history(self, body, kind):
        stack, other = (self.undo, self.redo) if kind == "undo" else (self.redo, self.undo)
        if not stack:
            raise Refusal(409, "nothing_to_undo", f"there is nothing to {kind}")
        entry = stack.pop()
        layouts = self.layouts()
        self.begin()
        self.restore(entry["before"] if kind == "undo" else entry["after"])
        self.touched = list(entry["touched"])
        self.structural = set(entry["structural"])
        self.sheet_list = entry["sheets"]
        self.evaluated = 0
        other.append(entry)
        return self.commit(None, layouts)

    def calculate(self, body):
        """POST calculate: every formula evaluated again; nothing waits any more."""
        if not isinstance(body, dict):
            raise Refusal(400, "invalid_record", "$: expected {full}")
        layouts = self.layouts()
        self.begin()
        count = 0
        for key, sheet in self.sheets.items():
            for item in sheet.cells.values():
                if not item["flags"] & 4:
                    continue
                before = (item["text"], item.get("value"), item["flags"])
                item["value"] = 0.0 if refers_to_itself(item) else Evaluator(self, key).run(item["entry"])
                item["flags"] &= ~8
                self.display(item)
                if (item["text"], item.get("value"), item["flags"]) != before:
                    count += 1
                    self.touched.append((key, (item["r"], item["c"], item["r"], item["c"])))
        self.workbook.setdefault("calc", {})["uncomputed"] = 0
        self.evaluated = count
        answer = self.commit(None, layouts)
        answer["undoable"] = False
        return answer

    def changes(self, since):
        if self.ring and since < self.ring[0]["revision"] - 1:
            return {"revision": self.workbook["revision"], "reset": True}
        return {"revision": self.workbook["revision"],
                "changes": [{k: v for k, v in c.items() if k != "touched"} for c in self.ring if c["revision"] > since]}


def split_tsv(text):
    """Rows of cells from Excel's text form: tabs, CRLF or LF rows, quoted cells."""
    rows, row, cell, i, n = [], [], "", 0, len(text)
    quoted = False
    while i < n:
        ch = text[i]
        if quoted:
            if ch == '"' and i + 1 < n and text[i + 1] == '"':
                cell += '"'
                i += 2
                continue
            if ch == '"':
                quoted = False
            else:
                cell += ch
        elif ch == '"' and cell == "":
            quoted = True
        elif ch == "\t":
            row.append(cell)
            cell = ""
        elif ch in "\r\n":
            if ch == "\r" and i + 1 < n and text[i + 1] == "\n":
                i += 1
            row.append(cell)
            rows.append(row)
            row, cell = [], ""
        else:
            cell += ch
        i += 1
    if cell or row:
        row.append(cell)
        rows.append(row)
    return rows or [[""]]


def quote_sheet(name):
    return name if re.fullmatch(r"[A-Za-z_][\w.]*", name) else "'" + name.replace("'", "''") + "'"


def shift_references(formula, rows, at, count, insert):
    """A formula's references after rows or columns are inserted or removed (same sheet only)."""
    def move(m):
        col_abs, col, row_abs, row = m.group(1), m.group(2), m.group(3), m.group(4)
        r, c = int(row) - 1, col_index(col)
        index = r if rows else c
        if insert and index >= at:
            index += count
        elif not insert and at <= index < at + count:
            return "#REF!"
        elif not insert and index >= at + count:
            index -= count
        r, c = (index, c) if rows else (r, index)
        return f"{col_abs}{col_name(c)}{row_abs}{r + 1}"
    parts = re.split(r'("(?:[^"]|"")*")', formula)
    for i in range(0, len(parts), 2):
        parts[i] = re.sub(r"(?<![\w$!])(\$?)([A-Za-z]{1,3})(\$?)(\d{1,7})(?![\w(!])", move, parts[i])
    return "".join(parts)


def refers_to_itself(item):
    for m in re.finditer(r"(?<![\w!$])\$?([A-Za-z]{1,3})\$?(\d{1,7})(?::\$?([A-Za-z]{1,3})\$?(\d{1,7}))?(?![\w(!])",
                         item["entry"]):
        r0, c0 = int(m.group(2)) - 1, col_index(m.group(1))
        r1, c1 = (int(m.group(4)) - 1, col_index(m.group(3))) if m.group(3) else (r0, c0)
        if min(r0, r1) <= item["r"] <= max(r0, r1) and min(c0, c1) <= item["c"] <= max(c0, c1):
            return True
    return False
