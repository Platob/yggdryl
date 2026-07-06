"""Q4 — Interval matching: how many numbers fall inside the intervals.

Given a list of numbers and closed intervals, count the numbers covered by at
least one interval, and (optionally) how many intervals contain each number.

After merging the intervals we binary-search each number against the merged
starts: O((n + m) log m) instead of the naive O(n * m).
"""

import bisect
from typing import List, Tuple

Interval = Tuple[int, int]


def _merge(intervals: List[Interval]) -> List[Interval]:
    if not intervals:
        return []
    ordered = sorted(intervals)
    merged: List[List[int]] = [list(ordered[0])]
    for lo, hi in ordered[1:]:
        if lo <= merged[-1][1]:
            merged[-1][1] = max(merged[-1][1], hi)
        else:
            merged.append([lo, hi])
    return [(lo, hi) for lo, hi in merged]


def count_numbers_in_any_interval(numbers: List[int], intervals: List[Interval]) -> int:
    """Count numbers that lie inside at least one closed interval."""
    return len(numbers_in_any_interval(numbers, intervals))


def numbers_in_any_interval(numbers: List[int], intervals: List[Interval]) -> List[int]:
    """The subset of ``numbers`` covered by at least one closed interval."""
    merged = _merge(intervals)
    if not merged:
        return []
    starts = [lo for lo, _ in merged]
    hits = []
    for x in numbers:
        i = bisect.bisect_right(starts, x) - 1
        if i >= 0 and x <= merged[i][1]:
            hits.append(x)
    return hits


def count_intervals_containing(number: int, intervals: List[Interval]) -> int:
    """How many of the (possibly overlapping) intervals contain ``number``."""
    return sum(1 for lo, hi in intervals if lo <= number <= hi)
