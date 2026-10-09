#!/bin/bash
# P4 chain, disk-bounded: the workspace's own artifacts are cleaned between the heavy steps, since the session's disk allowance holds one lane's test binaries at a time.
cd /home/user/yggdryl
S=/tmp/claude-0/-home-user-yggdryl/09ea5bac-ef2f-52ce-b6c3-cfd3a196ec17/scratchpad
export CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 CARGO_INCREMENTAL=0
L=/tmp/claude-0/-home-user-yggdryl/09ea5bac-ef2f-52ce-b6c3-cfd3a196ec17/scratchpad/logs/chain_p4.log
step() { echo "== STEP $1 ==" >> $L; df -h / | tail -1 >> $L; }
clean() { cargo clean -p yggdryl -p yggdryl-cli -p yggdryl-python -p yggdryl-node >> $L 2>&1; echo "cleaned" >> $L; }
: > $L
until mkdir $S/cargo.lock 2>/dev/null; do sleep 10; done
trap 'rmdir $S/cargo.lock' EXIT
git log --oneline -1 >> $L
step fmt-check
cargo fmt --all -- --check >> $L 2>&1
echo "fmt-check exit $?" >> $L
step whole
cargo test -p yggdryl --all-targets --all-features --no-fail-fast >> $L 2>&1
echo "whole exit $?" >> $L
step clippy
cargo clippy --workspace --all-targets --all-features --no-deps -- -D warnings >> $L 2>&1
echo "clippy exit $?" >> $L
step cargo-doc
RUSTDOCFLAGS="-D warnings" cargo doc -p yggdryl --no-deps --all-features >> $L 2>&1
echo "cargo-doc exit $?" >> $L
rm -rf target/doc
clean
step rustdoc
cargo test -p yggdryl --doc >> $L 2>&1
echo "rustdoc exit $?" >> $L
step clippy-default
cargo clippy -p yggdryl --all-targets --no-deps -- -D warnings >> $L 2>&1
echo "clippy-default exit $?" >> $L
step cli-tests
cargo test -p yggdryl-cli --all-targets --no-fail-fast >> $L 2>&1
echo "cli-tests exit $?" >> $L
step whole-default
cargo test -p yggdryl --all-targets --no-fail-fast >> $L 2>&1
echo "whole-default exit $?" >> $L
clean
step python
V=python/.venv/bin/python
VIRTUAL_ENV=python/.venv $V -m maturin develop -m python/Cargo.toml >> $L 2>&1
echo "maturin exit $?" >> $L
$V -m pytest python/tests -q --no-header -p no:cacheprovider --deselect python/tests/test_spark_interop.py >> $L 2>&1
echo "pytest exit $?" >> $L
$V -m mypy --strict --config-file python/pyproject.toml python/yggdryl python/tests/typing_bindings.py python/tests/typing_fields.py >> $L 2>&1
echo "mypy exit $?" >> $L
step node
npm run --prefix node test:package:debug >> $L 2>&1
echo "node test:package:debug exit $?" >> $L
cargo build --locked -p yggdryl-cli >> $L 2>&1
echo "cli build exit $?" >> $L
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
$V scripts/check_api_inventory.py >> $L 2>&1
echo "inventory exit $?" >> $L
$V scripts/generate_internals.py --check >> $L 2>&1
echo "internals exit $?" >> $L
step docs-python
$V scripts/check_docs_examples.py --lang python >> $L 2>&1
echo "docs-python exit $?" >> $L
step docs-javascript
$V scripts/check_docs_examples.py --lang javascript >> $L 2>&1
echo "docs-javascript exit $?" >> $L
step docs-rust
cargo clean -p yggdryl >> $L 2>&1
$V scripts/check_docs_examples.py --lang rust >> $L 2>&1
echo "docs-rust exit $?" >> $L
step git-status
git status --short >> $L 2>&1
step end
echo "== CHAIN_DONE ==" >> $L
