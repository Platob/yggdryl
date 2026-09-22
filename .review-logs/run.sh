set -x
cd /home/user/yggdryl
echo "##### build"
npm run --prefix node build:debug 2>&1 | tail -6
echo "##### node --test (failures)"
node --test "node/tests/**/*.test.js" 2>&1 | tail -400 > /home/user/yggdryl/.review-logs/nodetest.log
grep -E "^# (tests|pass|fail)|^not ok" /home/user/yggdryl/.review-logs/nodetest.log | head -30
echo "##### DONE"
