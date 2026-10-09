#!/bin/sh
# A fresh copy of ef4e241a4 (the worktree's base), S4 simulated and committed.
set -e
D=/tmp/claude-0/-home-user-yggdryl/09ea5bac-ef2f-52ce-b6c3-cfd3a196ec17/scratchpad
rm -rf $D/s5/sim && mkdir -p $D/s5/sim
git -C /home/user/yggdryl-s5 archive ef4e241a4 | tar -x -C $D/s5/sim
cd $D/s5/sim && git init -q && git add -A && git -c user.email=s5@local -c user.name=s5 commit -q -m base
python3 -I $D/s4_move.py $D/s5/sim --no-git-lock --residue $D/s5/s4sim_residue.md > $D/s5/s4sim.log 2>&1
cd $D/s5/sim && git add -A && git -c user.email=s5@local -c user.name=s5 commit -q -m s4
echo done
