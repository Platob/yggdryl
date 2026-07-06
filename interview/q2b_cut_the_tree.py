"""Q2 (variant) — Tree: "Cut the Tree" (HackerRank).

Remove exactly one edge to split the tree into two parts and minimise the
absolute difference of their node-value sums. Cutting the edge above node ``u``
gives one part equal to the subtree of ``u``, so the difference is
``abs(total - 2 * subtree_sum(u))``. Minimise over every non-root node.

Nodes are ``0 .. n-1`` (0-indexed; HackerRank is 1-indexed — subtract 1 on
input). Iterative traversal, O(n).
"""

from collections import defaultdict
from typing import List, Tuple


def cut_the_tree(values: List[int], edges: List[Tuple[int, int]]) -> int:
    """Minimum absolute difference between the two halves after one cut."""
    n = len(values)
    if n <= 1:
        return 0  # no edge to cut

    adj = defaultdict(list)
    for a, b in edges:
        adj[a].append(b)
        adj[b].append(a)

    parent = [-1] * n
    order: List[int] = []
    seen = [False] * n
    stack = [0]
    seen[0] = True
    while stack:
        u = stack.pop()
        order.append(u)
        for v in adj[u]:
            if not seen[v]:
                seen[v] = True
                parent[v] = u
                stack.append(v)

    subtree = values[:]  # subtree[u] accumulates its descendants' values
    for u in reversed(order):
        if parent[u] != -1:
            subtree[parent[u]] += subtree[u]

    total = subtree[order[0]]  # == sum(values)
    best = float("inf")
    for u in range(n):
        if parent[u] != -1:
            best = min(best, abs(total - 2 * subtree[u]))
    return int(best)
