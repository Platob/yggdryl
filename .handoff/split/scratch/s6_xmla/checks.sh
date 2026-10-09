#!/bin/bash
# The define's checks over one tree: $1 the moved tree. No cargo.
T=$1
D=/tmp/claude-0/-home-user-yggdryl/09ea5bac-ef2f-52ce-b6c3-cfd3a196ec17/scratchpad
cd $T
files=$( (git diff HEAD --name-only --diff-filter=AMR -M; git ls-files --others --exclude-standard) | grep '\.rs$' | sort -u)
n=0; perr=0; fdiff=0
for f in $files; do [ -f "$f" ] || continue; n=$((n+1)); out=$(rustfmt --edition 2024 --check "$f" 2>&1 >/dev/null); if echo "$out" | grep -q "^error"; then perr=$((perr+1)); echo "PARSE: $f"; echo "$out" | head -5; fi; if ! rustfmt --edition 2024 --check "$f" >/dev/null 2>&1; then fdiff=$((fdiff+1)); fi; done
echo "rustfmt: files=$n parse_errors=$perr format_diffs=$fdiff"
before=$(git grep -h -o '#\[test\]' HEAD -- ':(glob)rust/**/tests/**' 'cli/tests' | wc -l)
after_core=$(grep -rhoE '#\[test\]' rust/tests | wc -l)
after_xmla=$(grep -rhoE '#\[test\]' rust/xmla/tests 2>/dev/null | wc -l)
after_leaves=$(for d in rust/*/tests; do case $d in rust/tests|rust/xmla/tests) ;; *) grep -rhoE '#\[test\]' $d;; esac; done | wc -l)
after_cli=$(grep -rhoE '#\[test\]' cli/tests | wc -l)
echo "tests: before(all crates+cli)=$before after core=$after_core xmla=$after_xmla other-leaves=$after_leaves cli=$after_cli sum=$((after_core+after_xmla+after_leaves+after_cli))"
echo "src test code: $(grep -rn '#\[cfg(test)\]\|#\[test\]\|mod tests' rust/src rust/xmla/src python/src node/src cli/src | wc -l)"
python3 scripts/generate_internals.py --check; echo "generate_internals exit=$?"
python3 scripts/check_api_inventory.py; echo "check_api_inventory exit=$?"
python3 -m unittest discover -s scripts/tests -p test_ci_plan.py 2>&1 | tail -3
python3 $D/s6_avro/plan_check.py .
echo "model ids: $(git grep -n -i -E 'claude-(opus|sonnet|haiku|fable)|opus [0-9]|sonnet [0-9]|fable [0-9]' -- . ':!*.lock' | wc -l) tracked; untracked: $(git ls-files --others --exclude-standard | xargs -r grep -l -i -E 'claude-(opus|sonnet|haiku|fable)|opus [0-9]|sonnet [0-9]|fable [0-9]' 2>/dev/null | wc -l)"
echo "old paths: $(git grep -n -E 'yggdryl::(xmla|soap)\b|crate::xmla\b|crate::soap\b' -- rust cli python/src node/src docs skills AGENTS.md README.md .api-inventory.txt ':!rust/xmla/src' | wc -l) (outside the crate); crate::soap inside the crate is its own"
echo "status lines: $(git status --short | wc -l)"
echo "diff: $(git diff HEAD -M --stat | tail -1)"
find scripts -name __pycache__ -type d -prune -exec rm -rf {} +
