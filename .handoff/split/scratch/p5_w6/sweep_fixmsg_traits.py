#!/usr/bin/env python3
"""Re-spell the graph trait calls a test makes on a `FixMsg` onto its message.

D37 deletes `FixMsg`'s `Element`/`Event`/`Market`/`Operation` impls: a fact
is read through `message()` and stated through `message_mut()`. This sweep
tracks, per function, which local names hold a `FixMsg` (or a `Vec` of them)
from the expressions that bind them, and inserts `.message()` /
`.message_mut()` between such a receiver and a trait method. What it cannot
type it leaves for the compiler's list (`fix_e0599.py`).

usage: sweep_fixmsg_traits.py FILE...   (edits in place, prints a summary)
"""
import re
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
TRAIT_FNS = set((HERE.parent / "p5_w6_traitfns.txt").read_text().split())
# Inherent FixMsg methods sharing a trait method's name: left alone.
INHERENT = {"is_execution", "digest"}
CONSUMING = {
    "with_previous", "merge_with", "restating", "following", "following_market",
    "following_operation", "merging", "merging_market", "merging_market_event",
    "merging_operation", "merging_operation_event",
}
MUTATING_PREFIX = ("set_", "insert_", "remove_")
MUTATING = {
    "derive_securityid", "finalize", "finalized", "fill_market", "sync_cross",
    "follow_identity", "note_conflict", "fold_lifecycle",
}
METHODS = sorted(TRAIT_FNS - INHERENT, key=len, reverse=True)

SINGLE_PRODUCERS = re.compile(
    r"parse_fix_line\(|parse_ullink_line\(|parse_fixml_line\(|sole_line\(|sole_message\(|"
    r"dated_line\(|FixMsg::(?:new|with_registry|from_row|from_message)\("
)
NOT_A_MESSAGE = re.compile(
    r"into_market_data|\.market_data\(|into_market_leaf|MarketData::|into_message|"
    r"arrow_reader|_serie\(|into_row|into_bytes|into_text|\.get_\w+\(|by_tag|by_name|by_id|"
    r"by_path|\.entries\(|\.header\(|\.capture\(|\.anomalies\(|\.metadata\(|\.len\(\)|"
    r"\.is_\w+\(|==|\bmap\(|\.iter\(\)\.map|\.count\(\)|\.digest\(|\.stable_hash\(|\.registry\(|"
    r"\.text\(|\.carried\(|\.as_field\(|\.as_value\(|\.value\(|\.get\("
)


def helpers(text):
    single, many = set(), set()
    for m in re.finditer(r"fn (\w+)(?:<[^>]*>)?\([^)]*\)\s*->\s*([^{;]+)", text):
        ret = m.group(2).replace("yggdryl::", "").replace(" ", "")
        if re.fullmatch(r"(?:Result<)?FixMsg>?", ret):
            single.add(m.group(1))
        elif re.fullmatch(r"(?:Result<)?Vec<FixMsg>>?", ret):
            many.add(m.group(1))
    return single, many


def statement_end(text, start):
    depth = 0
    i = start
    in_str = False
    while i < len(text):
        c = text[i]
        if in_str:
            if c == "\\":
                i += 2
                continue
            if c == '"':
                in_str = False
        elif c == '"':
            in_str = True
        elif c in "([{":
            depth += 1
        elif c in ")]}":
            depth -= 1
            if depth < 0:
                return i
        elif c == ";" and depth == 0:
            return i
        i += 1
    return i


def skeleton(text):
    """`text` with what every call's parentheses and every string hold
    taken out: the chain a value is the answer of."""
    out, depth, i, in_str = [], 0, 0, False
    while i < len(text):
        c = text[i]
        if in_str:
            if c == "\\":
                i += 2
                continue
            if c == '"':
                in_str = False
        elif c == '"':
            in_str = True
        elif c == "(":
            if depth == 0:
                out.append(c)
            depth += 1
        elif c == ")":
            depth -= 1
            if depth == 0:
                out.append(c)
        elif depth == 0:
            out.append(c)
        i += 1
    return "".join(out)


def classify(rhs, scope, single, many):
    rhs = skeleton(rhs.strip())
    if NOT_A_MESSAGE.search(rhs):
        return None
    m = re.fullmatch(r"&?(?:mut\s+)?(\w+)(?:\.clone\(\))?", rhs)
    if m and m.group(1) in scope:
        return scope[m.group(1)]
    m = re.match(r"&?(\w+)\s*(?:\[[^\]]+\]|\.remove\(|\.pop\(|\.swap_remove\(|\.into_iter\(\)\s*\.next\(|"
                 r"\.first\(|\.last\(|\.iter\(\)\s*\.next\(|\.iter\(\)\s*\.nth\(|\.into_iter\(\)\s*\.nth\()", rhs)
    if m and scope.get(m.group(1)) == "vec":
        return "msg"
    calls = set(re.findall(r"\b(\w+)\(", rhs))
    first = re.match(r"&?(\w+)\(", rhs)
    if first and scope.get(first.group(1)) in ("fn_msg", "fn_vec"):
        held = scope[first.group(1)]
        if held == "fn_msg":
            return "msg"
        return "msg" if re.search(r"\.remove\(|\[\d+\]|\.pop\(|\.next\(", rhs) else "vec"
    if calls & many and ".collect" not in rhs and "[" not in rhs and "remove(" not in rhs:
        return "vec"
    if calls & many:
        return "msg" if re.search(r"\.remove\(|\[\d+\]|\.pop\(|\.next\(", rhs) else "vec"
    if SINGLE_PRODUCERS.search(rhs) or calls & single:
        return "vec" if ".collect" in rhs else "msg"
    if re.search(r"\.parse_line\(|\.parse_text_line\(|\.lifecycle\(|\.messages\(", rhs):
        if ".collect" in rhs:
            return "vec"
        if re.search(r"\.next\(\)", rhs):
            return "msg"
    return None


def expression_before(text, end):
    """The method-chain expression ending just before `end` (a `)`)."""
    i = end - 1
    while True:
        if text[i] != ")":
            return None
        depth = 0
        while i >= 0:
            if text[i] == ")":
                depth += 1
            elif text[i] == "(":
                depth -= 1
                if depth == 0:
                    break
            i -= 1
        j = i - 1
        while j >= 0 and (text[j].isalnum() or text[j] in "_:!"):
            j -= 1
        callee_start = j + 1
        k = j
        while k >= 0 and text[k] in " \n\t":
            k -= 1
        if k >= 0 and text[k] == ".":
            k -= 1
            while k >= 0 and text[k] in " \n\t":
                k -= 1
            if text[k] == ")":
                i = k
                continue
            j = k
            while j >= 0 and (text[j].isalnum() or text[j] in "_:"):
                j -= 1
            return text[j + 1:end]
        return text[callee_start:end]


def sweep(path):
    text = path.read_text()
    harness = (path.parent.parent / "fix.rs")
    hs, hm = helpers(harness.read_text()) if harness.exists() else (set(), set())
    single, many = helpers(text)
    single |= hs
    many |= hm
    events = []  # (pos, kind, payload)
    for m in re.finditer(r"\bfn \w+", text):
        events.append((m.start(), "fn", m))
    for m in re.finditer(r"\blet\s+(mut\s+)?(\w+)\s*(?::\s*([^=;]+?))?\s*=(?!=)", text):
        events.append((m.start(), "let", m))
    for m in re.finditer(r"\blet\s+\(([^)]*)\)\s*=\s*\(", text):
        events.append((m.start(), "tuple", m))
    for m in re.finditer(r"\blet\s+\[([^\]]*)\]\s*=\s*([^;]+?)(?:\s+else|;)", text):
        events.append((m.start(), "slice", m))
    for m in re.finditer(r"\bfor\s+(?:\(\w+,\s*)?(\w+)\)?\s+in\s+([^{]+)\{", text):
        events.append((m.start(), "for", m))
    for m in re.finditer(r"\blet\s+Some\(\((\w+),\s*(\w+)\)\)\s*=\s*&?(\w+)\.split_(?:first|last)\(\)", text):
        events.append((m.start(), "split", m))
    # The item a chain over a vector of messages lends its first closures.
    for m in re.finditer(r"\b(\w+)\s*\.(?:iter|into_iter|windows\(\d+\))\(?\)?", text):
        events.append((m.start(), "chain", m))
    for m in re.finditer(r"[(,|]\s*(mut\s+)?(\w+)\s*:\s*&?(?:mut\s+)?(?:yggdryl::)?(FixMsg|Vec<(?:yggdryl::)?FixMsg>)\b", text):
        events.append((m.start(), "param", m))
    pat = re.compile(
        r"(\b\w+|\))((?:\[[^\]\n]*\])?)(\s*)\.(%s)\(" % "|".join(METHODS)
    )
    for m in pat.finditer(text):
        events.append((m.start(), "call", m))
    events.sort(key=lambda e: (e[0], e[1] != "fn"))
    scope = {}
    edits = []
    skipped = []
    for pos, kind, m in events:
        if kind == "fn":
            scope = {}
        elif kind == "param":
            scope[m.group(2)] = "vec" if m.group(3).startswith("Vec") else "msg"
        elif kind == "let":
            name, ann = m.group(2), m.group(3)
            if ann:
                ann = ann.replace("yggdryl::", "").replace(" ", "")
                scope[name] = {"FixMsg": "msg", "Vec<FixMsg>": "vec"}.get(ann)
                continue
            end = statement_end(text, m.end())
            rhs = text[m.end():end].strip()
            closure = re.match(r"(?:move\s*)?\|[^|]*\|\s*(?:->\s*[^{]+)?", rhs)
            if closure:
                body = rhs[closure.end():]
                ret = re.search(r"->\s*(?:yggdryl::)?(FixMsg|Vec<(?:yggdryl::)?FixMsg>)", closure.group(0))
                held = ("vec" if ret.group(1).startswith("Vec") else "msg") if ret else classify(body.strip().strip("{}"), scope, single, many)
                scope[name] = {"msg": "fn_msg", "vec": "fn_vec"}.get(held)
            else:
                scope[name] = classify(rhs, scope, single, many)
        elif kind == "tuple":
            names = [n.strip().removeprefix("mut ").strip() for n in m.group(1).split(",")]
            end = statement_end(text, m.end() - 1)
            inner = text[m.end():end].rstrip().removesuffix(")")
            parts, depth, cur = [], 0, ""
            for c in inner:
                if c in "([{": depth += 1
                if c in ")]}": depth -= 1
                if c == "," and depth == 0:
                    parts.append(cur); cur = ""
                else:
                    cur += c
            parts.append(cur)
            for name, part in zip(names, parts):
                if re.fullmatch(r"\w+", name):
                    scope[name] = classify(part, scope, single, many)
        elif kind == "slice":
            src = m.group(2)
            base = re.match(r"&?(\w+)", src.strip())
            is_vec = (base and scope.get(base.group(1)) == "vec") or bool(
                re.match(r"\s*<\[(?:yggdryl::)?FixMsg\b", src)
            ) or bool(base and (base.group(1) in many or scope.get(base.group(1)) == "fn_vec"))
            for name in re.findall(r"\b([a-z_]\w*)\b", m.group(1)):
                if name not in ("mut", "ref"):
                    scope[name] = "msg" if is_vec else None
        elif kind == "for":
            src = m.group(2).strip()
            items = re.fullmatch(r"\[(.*)\]", src, re.S)
            held = None
            if items:
                parts = [p.strip().lstrip("&").strip() for p in items.group(1).split(",") if p.strip()]
                if parts and all(scope.get(p) == "msg" for p in parts):
                    held = "msg"
            else:
                base = re.match(r"&?(?:mut\s+)?(\w+)", src)
                plain = re.fullmatch(r"&?(?:mut\s+)?(\w+)(?:\[[^\]]*\])?(?:\.iter\(\)|\.into_iter\(\)|\.iter_mut\(\))?", src)
                if plain and scope.get(plain.group(1)) == "vec":
                    held = "msg"
                elif base and (base.group(1) in many or scope.get(base.group(1)) == "fn_vec") and "map" not in src:
                    held = "msg"
            scope[m.group(1)] = held
        elif kind == "split":
            if scope.get(m.group(3)) == "vec":
                scope[m.group(1)] = "msg"
                scope[m.group(2)] = "vec"
        elif kind == "chain":
            if scope.get(m.group(1)) != "vec":
                continue
            rest = text[m.end():m.end() + 600]
            for adapter in re.finditer(r"\.(\w+)\(\s*(?:move\s*)?\|&?\(?(\w+)", rest):
                if adapter.group(1) in ("filter", "find", "any", "all", "position", "rfind", "take_while", "skip_while", "inspect", "for_each", "map", "find_map", "filter_map", "flat_map", "partition", "max_by_key", "min_by_key"):
                    if adapter.group(1) in ("filter", "find", "rfind", "take_while", "skip_while", "inspect"):
                        scope[adapter.group(2)] = "ref_msg"
                    else:
                        scope[adapter.group(2)] = "msg"
                    if adapter.group(1) in ("map", "find_map", "filter_map", "flat_map", "for_each", "any", "all", "position", "partition", "max_by_key", "min_by_key"):
                        break
                else:
                    break
        elif kind == "call":
            name, index, ws, method = m.group(1), m.group(2), m.group(3), m.group(4)
            if name == ")":
                if re.search(r"\.message(_mut)?\(\s*$", text[max(0, m.start() - 20):m.start()]):
                    continue
                expr = expression_before(text, m.start() + 1)
                held = classify(expr, scope, single, many) if expr else None
                if held != "msg":
                    continue
                index = ""
            else:
                held = scope.get(name)
            if held == "ref_msg":
                held = "msg"
            if not ((held == "msg" and not index) or (held == "vec" and index)):
                continue
            line = text.count("\n", 0, m.start()) + 1
            if method in CONSUMING:
                skipped.append((line, method))
                continue
            door = ".message_mut()" if (method.startswith(MUTATING_PREFIX) or method in MUTATING) else ".message()"
            at = m.start(3)
            edits.append((at, door))
    for at, door in sorted(edits, reverse=True):
        text = text[:at] + door + text[at:]
    path.write_text(text)
    return len(edits), skipped


if __name__ == "__main__":
    for arg in sys.argv[1:]:
        count, skipped = sweep(Path(arg))
        print(f"{arg}: {count} calls re-spelled; consuming calls left: {skipped}")
