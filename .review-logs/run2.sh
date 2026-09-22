set -x
cd /home/user/yggdryl
echo "##### test:package:debug"
npm run --prefix node test:package:debug 2>&1 | tail -8
echo "##### clippy"
cargo clippy -p yggdryl-node --all-targets 2>&1 | tail -12
echo "##### fmt"
cargo fmt -p yggdryl-node -- --check 2>&1 | tail -10
echo "##### npm test (tests + tsc)"
npm test --prefix node 2>&1 | grep -E "^# (tests|pass|fail|duration)|^not ok|error TS" | head -20
echo "##### playground"
node scripts/build_docs_playground.js --check 2>&1 | tail -3
echo "##### fix docs"
node scripts/build_docs_fix.js --check 2>&1 | tail -3
echo "##### docs examples javascript"
python3 scripts/check_docs_examples.py --lang javascript 2>&1 | tail -12
echo "##### generated files vs HEAD"
git diff --stat -- node/index.js node/index.d.ts
echo "##### DONE"
