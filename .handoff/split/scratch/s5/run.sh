#!/bin/sh
# A copy of the S4-simulated tree, then the S5 move.
set -e
D=/tmp/claude-0/-home-user-yggdryl/09ea5bac-ef2f-52ce-b6c3-cfd3a196ec17/scratchpad
rm -rf $D/s5/run && cp -a $D/s5/sim $D/s5/run
python3 -I $D/s5_move.py $D/s5/run --no-git-lock --residue $D/s5/run_residue.md
