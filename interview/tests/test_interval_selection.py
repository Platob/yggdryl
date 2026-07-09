import random
from itertools import combinations

from interval_selection import interval_selection


def _max_coverage(combo):
    points = set()
    for lo, hi in combo:
        points.add(lo)
        points.add(hi)
    return max(
        (sum(1 for lo, hi in combo if lo <= x <= hi) for x in points),
        default=0,
    )


def _brute_force(intervals, k):
    for size in range(len(intervals), 0, -1):
        for combo in combinations(intervals, size):
            if _max_coverage(combo) <= k:
                return size
    return 0


def test_examples():
    assert interval_selection([(1, 2), (2, 3), (1, 3)], k=2) == 2
    assert interval_selection([(1, 3), (2, 4), (3, 5)], k=1) == 1
    assert interval_selection([], k=2) == 0


def test_all_fit_when_disjoint():
    assert interval_selection([(1, 2), (3, 4), (5, 6)], k=1) == 3


def test_matches_brute_force_random():
    rng = random.Random(1234)
    for _ in range(200):
        n = rng.randint(0, 7)
        intervals = []
        for _ in range(n):
            lo = rng.randint(0, 8)
            hi = lo + rng.randint(0, 5)
            intervals.append((lo, hi))
        for k in (1, 2, 3):
            assert interval_selection(intervals, k) == _brute_force(intervals, k)
