"""Q3 — Segment intersections (interval merging & overlap).

Closed segments such as ``[3, 5]`` and ``[4, 6]``. Touching endpoints count as
overlapping (``[1, 3]`` and ``[3, 5]`` merge into ``[1, 5]``).

- ``merge_intervals``       — union of overlapping segments, O(n log n).
- ``total_covered_length``  — length of the union.
- ``max_overlap``           — greatest number of segments covering any point
                              (sweep line), O(n log n).
"""

from typing import List, Tuple

Interval = Tuple[int, int]


def merge_intervals(intervals: List[Interval]) -> List[Interval]:
    """Merge overlapping/touching closed intervals into a disjoint sorted list."""
    if not intervals:
        return []
    ordered = sorted(intervals)
    merged: List[List[int]] = [list(ordered[0])]
    for lo, hi in ordered[1:]:
        if lo <= merged[-1][1]:  # overlap or touch
            merged[-1][1] = max(merged[-1][1], hi)
        else:
            merged.append([lo, hi])
    return [(lo, hi) for lo, hi in merged]


def total_covered_length(intervals: List[Interval]) -> int:
    """Total length covered by the union of the intervals."""
    return sum(hi - lo for lo, hi in merge_intervals(intervals))


def max_overlap(intervals: List[Interval]) -> int:
    """Maximum number of intervals overlapping at a single point."""
    events: List[Tuple[int, int]] = []
    for lo, hi in intervals:
        events.append((lo, 1))
        events.append((hi, -1))
    # At equal coordinates process starts (+1) before ends (-1) so that
    # touching endpoints count as an overlap (closed intervals).
    events.sort(key=lambda e: (e[0], -e[1]))
    current = best = 0
    for _, delta in events:
        current += delta
        best = max(best, current)
    return best
