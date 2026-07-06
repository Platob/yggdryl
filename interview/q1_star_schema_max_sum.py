"""Q1 — Graph: "Star Schema" with maximum sum.

LeetCode 2497 (Maximum Star Sum of a Graph). A *star* is a centre node plus up
to ``k`` of its neighbours. Its sum is the centre value plus the values of the
chosen neighbours. Only neighbours with a positive value ever help, so for each
node we take its ``k`` largest positive neighbour values.

Complexity: O(E + V * d log d) where d is the max degree (sorting each
adjacency list). A heap of size k would drop the ``log d`` to ``log k``.
"""

from collections import defaultdict
from typing import List


def max_star_sum(vals: List[int], edges: List[List[int]], k: int) -> int:
    """Return the maximum star sum with a centre and at most ``k`` neighbours."""
    neighbours = defaultdict(list)
    for a, b in edges:
        neighbours[a].append(vals[b])
        neighbours[b].append(vals[a])

    best = max(vals)  # a lone centre (no neighbours) is always allowed
    for node in range(len(vals)):
        total = vals[node]
        for value in sorted(neighbours[node], reverse=True)[:k]:
            if value <= 0:
                break  # sorted descending, nothing positive left
            total += value
        best = max(best, total)
    return best
