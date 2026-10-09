#!/bin/bash
# P6+S4 chain, disk-bounded: the workspace's own test artifacts are cleaned between the heavy
# steps, since the session's disk holds one lane's test binaries at a time. Per package rather
# than --workspace for the test runs: the two binding crates are cdylibs whose test harness
# does not link, as CI runs them (ci.yml: -p yggdryl, -p <leaf>, -p yggdryl-cli).
cd /home/user/yggdryl
S=/tmp/s
export CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 CARGO_INCREMENTAL=0
L=$S/logs/chain_s4.log
step() { echo "== STEP $1 == $(date -u +%H:%M:%S)" >> $L; df -h / | tail -1 >> $L; }
clean() { cargo clean -p yggdryl -p yggdryl-market -p yggdryl-fix -p yggdryl-cli -p yggdryl-python -p yggdryl-node >> $L 2>&1; echo "cleaned" >> $L; }
: > $L
git log --oneline -3 >> $L
step fmt-check
cargo fmt --all -- --check >> $L 2>&1
echo "fmt-check exit $?" >> $L
step whole-core-all-features
cargo test --locked -p yggdryl --all-targets --all-features --no-fail-fast >> $L 2>&1
echo "whole-core exit $?" >> $L
step whole-market-all-features
cargo test --locked -p yggdryl-market --all-targets --all-features --no-fail-fast >> $L 2>&1
echo "whole-market exit $?" >> $L
step whole-fix-all-features
cargo test --locked -p yggdryl-fix --all-targets --all-features --no-fail-fast >> $L 2>&1
echo "whole-fix exit $?" >> $L
step clippy-all-features
cargo clippy --locked --workspace --all-targets --all-features --no-deps -- -D warnings >> $L 2>&1
echo "clippy exit $?" >> $L
step cargo-doc
RUSTDOCFLAGS="-D warnings" cargo doc --locked --workspace --no-deps --all-features >> $L 2>&1
echo "cargo-doc exit $?" >> $L
rm -rf target/doc
step bench-check
cargo check --locked -p yggdryl --profile bench --benches --all-features >> $L 2>&1
echo "bench-check-core exit $?" >> $L
cargo check --locked -p yggdryl-market --profile bench --benches --all-features >> $L 2>&1
echo "bench-check-market exit $?" >> $L
cargo check --locked -p yggdryl-fix --profile bench --benches --all-features >> $L 2>&1
echo "bench-check-fix exit $?" >> $L
clean
step rustdoc
cargo test --locked -p yggdryl --doc >> $L 2>&1
echo "rustdoc-core exit $?" >> $L
cargo test --locked -p yggdryl --doc --all-features >> $L 2>&1
echo "rustdoc-core-all exit $?" >> $L
cargo test --locked -p yggdryl-market --doc --all-features >> $L 2>&1
echo "rustdoc-market exit $?" >> $L
cargo test --locked -p yggdryl-fix --doc --all-features >> $L 2>&1
echo "rustdoc-fix exit $?" >> $L
step clippy-default
cargo clippy --locked -p yggdryl --all-targets --no-deps -- -D warnings >> $L 2>&1
echo "clippy-default-core exit $?" >> $L
cargo clippy --locked -p yggdryl-market --all-targets --no-deps -- -D warnings >> $L 2>&1
echo "clippy-default-market exit $?" >> $L
cargo clippy --locked -p yggdryl-fix --all-targets --no-deps -- -D warnings >> $L 2>&1
echo "clippy-default-fix exit $?" >> $L
step cli-tests
cargo test --locked -p yggdryl-cli --all-targets --no-fail-fast >> $L 2>&1
echo "cli-tests exit $?" >> $L
step whole-default
cargo test --locked -p yggdryl --all-targets --no-fail-fast >> $L 2>&1
echo "whole-core-default exit $?" >> $L
cargo test --locked -p yggdryl-market --all-targets --no-fail-fast >> $L 2>&1
echo "whole-market-default exit $?" >> $L
cargo test --locked -p yggdryl-fix --all-targets --no-fail-fast >> $L 2>&1
echo "whole-fix-default exit $?" >> $L
clean
step python
V=python/.venv/bin/python
VIRTUAL_ENV=python/.venv $V -m maturin develop -m python/Cargo.toml >> $L 2>&1
echo "maturin exit $?" >> $L
$V -m pytest python/tests/test_fix.py python/tests/test_isin_registry.py -k medallion -q --no-header -p no:cacheprovider >> $L 2>&1
echo "pytest-medallion exit $?" >> $L
$V -m pytest python/tests -q --no-header -p no:cacheprovider --deselect python/tests/test_spark_interop.py >> $L 2>&1
echo "pytest exit $?" >> $L
$V -m mypy --strict --config-file python/pyproject.toml python/yggdryl python/tests/typing_bindings.py python/tests/typing_fields.py >> $L 2>&1
echo "mypy exit $?" >> $L
step node
npm run --prefix node test:package:debug >> $L 2>&1
echo "node test:package:debug exit $?" >> $L
npm test --prefix node >> $L 2>&1
echo "node test exit $?" >> $L
(cd node && npx tsc --noEmit) >> $L 2>&1
echo "tsc exit $?" >> $L
git diff --stat -- node/index.js node/index.d.ts >> $L 2>&1
echo "node generated diff exit $?" >> $L
step manifests
node scripts/build_docs_fix.js --check >> $L 2>&1
echo "fix manifest exit $?" >> $L
node scripts/build_docs_playground.js --check >> $L 2>&1
echo "playground manifest exit $?" >> $L
step mkdocs
$V -m mkdocs build --strict --config-file mkdocs.yml >> $L 2>&1
echo "mkdocs exit $?" >> $L
rm -rf site
step inventory
python3 scripts/check_api_inventory.py >> $L 2>&1
echo "inventory exit $?" >> $L
python3 scripts/generate_internals.py --check >> $L 2>&1
echo "internals exit $?" >> $L
python3 -m unittest discover -s scripts/tests -p test_ci_plan.py >> $L 2>&1
echo "planner exit $?" >> $L
step greps
grep -rn '#\[cfg(test)\]\|#\[test\]\|mod tests' rust/src rust/*/src python/src node/src cli/src >> $L 2>&1
echo "test-in-src grep exit $? (1 = clean)" >> $L
MODEL_WORDS="op""us|fa""ble|son""net|cla""ude-[a-z]+-[0-9]"
grep -rniE "$MODEL_WORDS" --exclude-dir=.git --exclude-dir=target --exclude-dir=node_modules --exclude-dir=.handoff --exclude-dir=.venv --exclude-dir=site . >> $L 2>&1
echo "model-id grep exit $? (1 = clean)" >> $L
step docs-python
$V scripts/check_docs_examples.py --lang python >> $L 2>&1
echo "docs-python exit $?" >> $L
step docs-javascript
$V scripts/check_docs_examples.py --lang javascript >> $L 2>&1
echo "docs-javascript exit $?" >> $L
step docs-rust
$V scripts/check_docs_examples.py --lang rust >> $L 2>&1
echo "docs-rust exit $?" >> $L
step git-status
git status --short >> $L 2>&1
step end
echo "== CHAIN_DONE == $(date -u +%H:%M:%S)" >> $L
