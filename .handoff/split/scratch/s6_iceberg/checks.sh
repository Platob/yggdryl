#!/usr/bin/env bash
# Usage: checks.sh <tree> <base-rev>   (no cargo)
T="$1"; BASE="${2:-HEAD}"; S=/tmp/claude-0/-home-user-yggdryl/09ea5bac-ef2f-52ce-b6c3-cfd3a196ec17/scratchpad
cd "$T" || exit 1
echo "== generate_internals --check"; python3 -I scripts/generate_internals.py --check; echo "rc=$?"
echo "== check_api_inventory"; python3 -I scripts/check_api_inventory.py 2>&1 | tail -1; echo "rc=${PIPESTATUS[0]}"
echo "== test_ci_plan"; python3 -m unittest discover -s scripts/tests -p test_ci_plan.py 2>&1 | tail -3 | tr '\n' ' '; echo
echo "== plan.check"; python3 -I -c "
import sys, pathlib
sys.path.insert(0, 'scripts/ci')
import plan
cfg = plan.load_config(); wf = plan.load_workflow()
print('check:', plan.check(cfg, wf, pathlib.Path('.').resolve()), 'leaves:', plan.check_leaves(cfg, pathlib.Path('.').resolve()))
"
echo "== test code under src"; grep -rlnE '#\[(cfg\()?test\b|#\[test\]' --include=*.rs rust/src rust/*/src 2>/dev/null | head; echo "(end)"
echo "== tests before/after"; python3 - "$BASE" <<'PY'
import re, subprocess, collections, pathlib, sys
base=sys.argv[1]
def names(text):
    out=[]
    for m in re.finditer(r'#\[test\]', text):
        fn=re.compile(r'\bfn\s+(\w+)').search(text, m.end())
        if fn: out.append(fn.group(1))
    return out
files=subprocess.run(['git','ls-tree','-r','--name-only',base,'rust','cli','python/src','node/src'],capture_output=True,text=True).stdout.split()
before=collections.Counter()
for f in files:
    if f.endswith('.rs'):
        before.update(names(subprocess.run(['git','show',f'{base}:{f}'],capture_output=True,text=True).stdout))
after=collections.Counter()
root=pathlib.Path('.')
for p in list(root.glob('rust/**/*.rs'))+list(root.glob('cli/**/*.rs'))+list(root.glob('python/src/**/*.rs'))+list(root.glob('node/src/**/*.rs')):
    if 'target' in p.parts: continue
    after.update(names(p.read_text()))
print('before', sum(before.values()), 'after', sum(after.values()), 'lost', dict(before-after), 'gained', sorted((after-before).elements()))
PY
echo "== status"; git status --short | wc -l; git diff HEAD -M --stat | tail -1
echo "== rustfmt parse"; git diff HEAD --name-only --diff-filter=AMR -M | grep '\.rs$' > /tmp/claude-0/-home-user-yggdryl/09ea5bac-ef2f-52ce-b6c3-cfd3a196ec17/scratchpad/s6_iceberg/_changed.txt
fails=0; n=0; while read f; do n=$((n+1)); out=$(rustfmt --edition 2024 --check "$f" 2>&1 >/dev/null); if echo "$out" | grep -q '^error'; then echo "PARSE: $f"; echo "$out" | head -3; fails=$((fails+1)); fi; done < $S/s6_iceberg/_changed.txt; echo "parse failures: $fails of $n"
echo "== moved-crate paths"; git grep -nE 'yggdryl::(avro|parquet|s3|iceberg|s3tables)::' -- . ':!.handoff' | grep -v 'yggdryl::iceberg::PrimitiveType' | cut -c1-160
