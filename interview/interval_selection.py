"""Extra — Interval Selection (HackerRank).

Select the maximum number of closed intervals so that no point on the line is
covered by more than ``k`` selected intervals (``k = 2`` in the original
problem). Greedy: sort by right endpoint, and take an interval whenever adding it
keeps the coverage over its whole span ``<= k``. This generalises earliest-
deadline-first (the classic ``k = 1`` activity selection) and is optimal by an
exchange argument.

Coverage is tracked with a lazy segment tree (range-add, range-max) over the
compressed endpoint coordinates: O(n log n).
"""

from typing import List, Tuple

Interval = Tuple[int, int]


class _SegTree:
    """Segment tree supporting range add and range max."""

    def __init__(self, size: int) -> None:
        self.size = size
        self.tree = [0] * (4 * size)
        self.lazy = [0] * (4 * size)

    def _push_down(self, node: int) -> None:
        if self.lazy[node]:
            for child in (2 * node, 2 * node + 1):
                self.tree[child] += self.lazy[node]
                self.lazy[child] += self.lazy[node]
            self.lazy[node] = 0

    def add(self, lo: int, hi: int, val: int, node: int = 1, nl: int = 0, nr: int = None) -> None:
        if nr is None:
            nr = self.size - 1
        if hi < nl or nr < lo:
            return
        if lo <= nl and nr <= hi:
            self.tree[node] += val
            self.lazy[node] += val
            return
        self._push_down(node)
        mid = (nl + nr) // 2
        self.add(lo, hi, val, 2 * node, nl, mid)
        self.add(lo, hi, val, 2 * node + 1, mid + 1, nr)
        self.tree[node] = max(self.tree[2 * node], self.tree[2 * node + 1])

    def max(self, lo: int, hi: int, node: int = 1, nl: int = 0, nr: int = None) -> int:
        if nr is None:
            nr = self.size - 1
        if hi < nl or nr < lo:
            return float("-inf")
        if lo <= nl and nr <= hi:
            return self.tree[node]
        self._push_down(node)
        mid = (nl + nr) // 2
        return max(
            self.max(lo, hi, 2 * node, nl, mid),
            self.max(lo, hi, 2 * node + 1, mid + 1, nr),
        )


def interval_selection(intervals: List[Interval], k: int = 2) -> int:
    """Maximum intervals selectable with no point covered more than ``k`` times."""
    if not intervals:
        return 0
    coords = sorted({p for interval in intervals for p in interval})
    index = {x: i for i, x in enumerate(coords)}
    tree = _SegTree(len(coords))

    chosen = 0
    for lo, hi in sorted(intervals, key=lambda iv: (iv[1], iv[0])):
        left, right = index[lo], index[hi]
        if tree.max(left, right) < k:
            tree.add(left, right, 1)
            chosen += 1
    return chosen
