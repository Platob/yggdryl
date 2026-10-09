#!/usr/bin/env python3
"""C3: re-spell the retired FixMsg market_data wrappers at the compiler's sites.
usage: c3_sites.py CHECK_LOG  (run from the repo root)"""
import re, sys, collections
from pathlib import Path
LINE = re.compile(r"^(?P<f>[^:\s]+\.rs):(?P<l>\d+):(?P<c>\d+): error\[E0599\]: no (?:method|associated function or constant) named `(?P<m>market_data|into_market_data|into_market_leaf)` found for (?:struct|reference) `&?FixMsg`")

def match_paren(s, i):
    # s[i] == '('
    depth = 0; j = i
    in_str = False
    while j < len(s):
        ch = s[j]
        if in_str:
            if ch == '\\': j += 2; continue
            if ch == '"': in_str = False
        elif ch == '"': in_str = True
        elif ch == '(': depth += 1
        elif ch == ')':
            depth -= 1
            if depth == 0: return j
        j += 1
    raise ValueError('unbalanced')

def drop_result(s, k):
    """s[k:] follows a call returning what was a Result and is now a Vec: drop .expect(..)/.unwrap()/?"""
    m = re.match(r"\s*\.expect\(", s[k:])
    if m:
        start = k + m.start() if False else k
        # keep whitespace? drop from first non-space '.'
        dot = k + m.group(0).index('.')
        open_ = k + m.end() - 1
        close = match_paren(s, open_)
        return s[:k] + s[close+1:], True
    m = re.match(r"\s*\.unwrap\(\)", s[k:])
    if m:
        return s[:k] + s[k+m.end():], True
    if s[k:k+1] == '?':
        return s[:k] + s[k+1:], True
    return s, False

def main(log):
    sites = collections.defaultdict(set)
    for t in Path(log).read_text().splitlines():
        m = LINE.match(t)
        if m: sites[m['f']].add((int(m['l']), int(m['c']), m['m']))
    manual = []
    for f, ss in sites.items():
        p = Path(f); s = p.read_text()
        lines = s.split('\n')
        offs = [0]
        for ln in lines: offs.append(offs[-1] + len(ln) + 1)
        for (l, c, meth) in sorted(ss, reverse=True):
            at = offs[l-1] + c - 1
            if s.startswith(meth + '(', at) and s[at-1] == '.':
                # method call form
                close = at + len(meth)
                assert s[close:close+2] == '()', (f, l, s[at:at+40])
                end = close + 2
                if meth == 'market_data':
                    new = 'clone().into_message().into_market_data()'
                else:
                    new = 'into_message().' + meth + '()'
                s = s[:at] + new + s[end:]
                end = at + len(new)
                if meth != 'into_market_leaf':
                    s, ok = drop_result(s, end)
                    if not ok: manual.append(f'{f}:{l}: no Result unwrap after {meth}')
            elif s.startswith(meth + '(', at) and s[at-2:at] == '::':
                # FixMsg::into_market_leaf(arg) / FixMsg::into_market_data(arg)
                head = s.rfind('FixMsg::', 0, at)
                open_ = at + len(meth)
                close = match_paren(s, open_)
                arg = s[open_+1:close]
                new = f'FixMsg::into_message({arg}).{meth}()'
                s = s[:head] + new + s[close+1:]
                if meth != 'into_market_leaf':
                    s, ok = drop_result(s, head + len(new))
                    if not ok: manual.append(f'{f}:{l}: no Result unwrap after {meth}')
            else:
                manual.append(f'{f}:{l}:{c}: {meth} in another form: {s[at-30:at+40]!r}')
            # recompute offsets (edits are right-to-left/bottom-up so earlier lines unaffected)
        p.write_text(s)
    print(sum(len(v) for v in sites.values()), 'sites in', len(sites), 'files')
    for m in manual: print('manual:', m)

main(sys.argv[1])
