#!/bin/sh
# A fresh copy of the program branch's ef4e241a4 (the define's base), then the move.
set -e
D=/tmp/claude-0/-home-user-yggdryl/09ea5bac-ef2f-52ce-b6c3-cfd3a196ec17/scratchpad
rm -rf $D/s6_xmla/sim && mkdir -p $D/s6_xmla/sim
git -C /home/user/yggdryl-s6-xmla archive ef4e241a4 | tar -x -C $D/s6_xmla/sim
cd $D/s6_xmla/sim && git init -q && git add -A && git -c user.email=s6@local -c user.name=s6 commit -q -m base
python3 -I $D/s6_xmla_move.py $D/s6_xmla/sim --no-git-lock --residue $D/s6_xmla/sim_residue.md
