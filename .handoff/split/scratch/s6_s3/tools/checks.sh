#!/bin/bash
# checks.sh <tree> <before-tree-or-git-ref-root>
T=$1; B=$2; S=/tmp/claude-0/-home-user-yggdryl/09ea5bac-ef2f-52ce-b6c3-cfd3a196ec17/scratchpad
cd $T
files=$(git status --porcelain | awk '{print $NF}' | grep '\.rs$')
n=0; perr=0; fmt=0
for f in $files; do [ -f "$f" ] || continue; n=$((n+1)); out=$(rustfmt --edition 2024 --check "$f" 2>&1 >/dev/null); rc=$?; if echo "$out" | grep -q '^error'; then perr=$((perr+1)); echo "PARSE $f"; echo "$out" | head -5; elif [ $rc -ne 0 ]; then fmt=$((fmt+1)); fi; done
echo "rustfmt: files=$n parse_errors=$perr fmt_diff=$fmt"
python3 $S/s6_s3/tools/pins.py $B . | head -4
echo "internals: $(python3 scripts/generate_internals.py --check 2>&1 | tail -1)"
echo "inventory: $(python3 scripts/check_api_inventory.py 2>&1 | tail -1)"
echo "planner: $(python3 -m unittest discover -s scripts/tests -p test_ci_plan.py 2>&1 | tail -1)"
python3 -c "
import sys, pathlib; sys.path.insert(0,'scripts/ci'); import plan
c=plan.load_config(); print('plan.check', plan.check(c, plan.load_workflow()), 'check_leaves', plan.check_leaves(c, pathlib.Path('.').resolve()), 'leaves', sorted(c.leaves) if hasattr(c,'leaves') else '?')"
echo "test code in src: $(grep -rn '#\[cfg(test)\]\|#\[test\]\|mod tests' rust/src rust/s3/src python/src node/src cli/src 2>/dev/null | wc -l)"
echo "old paths: $(git grep -n 'yggdryl::s3\b\|crate::s3::\|internals::s3_' -- . ':!.handoff' | wc -l)"
echo "s3 feature gates: $(git grep -n 'feature = "s3"' -- . ':!.handoff' ':!cli' | wc -l)"
echo "models: $(git grep -n -i -E 'claude-(opus|sonnet|haiku|fable)|opus [0-9]|sonnet [0-9]|fable [0-9]' | wc -l)"
echo "status: $(git status --short | wc -l)"
echo "diffstat: $(git diff HEAD -M --stat | tail -1)"
rm -rf scripts/ci/__pycache__ scripts/tests/__pycache__ scripts/__pycache__
