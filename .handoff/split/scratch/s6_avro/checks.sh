#!/bin/bash
# The define's checks over one tree: $1 the moved tree, $2 the base tree (for counts).
T=$1; B=$2
cd $T
files=$( (git diff HEAD --name-only --diff-filter=AMR -M; git ls-files --others --exclude-standard) | grep '\.rs$' | sort -u)
n=0; perr=0; fdiff=0
for f in $files; do [ -f "$f" ] || continue; n=$((n+1)); out=$(rustfmt --edition 2024 --check "$f" 2>&1 >/dev/null); if echo "$out" | grep -q "^error"; then perr=$((perr+1)); echo "PARSE: $f"; echo "$out" | head -5; fi; if ! rustfmt --edition 2024 --check "$f" >/dev/null 2>&1; then fdiff=$((fdiff+1)); fi; done
echo "rustfmt: files=$n parse_errors=$perr format_diffs=$fdiff"
echo "tests before: core=$(git show HEAD:rust/tests >/dev/null 2>&1; git grep -h -o '#\[test\]' HEAD -- rust/tests | wc -l) avro=$(git grep -h -o '#\[test\]' HEAD -- rust/avro/tests 2>/dev/null | wc -l)"
echo "tests after:  core=$(grep -rhoE '#\[test\]' rust/tests | wc -l) avro=$(grep -rhoE '#\[test\]' rust/avro/tests 2>/dev/null | wc -l)"
echo "src test code: $(grep -rn '#\[cfg(test)\]\|#\[test\]\|mod tests' rust/src rust/avro/src python/src node/src cli/src | wc -l)"
python3 scripts/generate_internals.py --check; echo "generate_internals exit=$?"
python3 scripts/check_api_inventory.py; echo "check_api_inventory exit=$?"
python3 -m unittest discover -s scripts/tests -p test_ci_plan.py 2>&1 | tail -3
python3 /tmp/s/s6_avro/plan_check.py .
echo "model ids: $(git grep -n -i -E 'claude-(opus|sonnet|haiku|fable)|opus [0-9]|sonnet [0-9]|fable [0-9]' -- . ':!*.lock' | wc -l) tracked; untracked: $(git ls-files --others --exclude-standard | xargs grep -l -i -E 'claude-(opus|sonnet|haiku|fable)|opus [0-9]|sonnet [0-9]|fable [0-9]' 2>/dev/null | wc -l)"
echo "status lines: $(git status --short | wc -l)"
echo "diff: $(git diff HEAD -M --stat | tail -1)"
