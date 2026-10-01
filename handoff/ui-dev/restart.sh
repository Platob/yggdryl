#!/bin/sh
# Restart the dev server on port 8765, remembering its pid.
cd "$(dirname "$0")"
[ -f dev.pid ] && kill "$(cat dev.pid)" 2>/dev/null
sleep 0.3
python3 dev.py --port 8765 "$@" > dev.log 2>&1 &
echo $! > dev.pid
sleep 0.8
head -1 dev.log
