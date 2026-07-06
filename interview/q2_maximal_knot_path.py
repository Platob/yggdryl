"""Q2 — Tree: maximal "knot" path.

Find the simple path in a tree that maximises the sum of node ("knot") values.
The path may start and end at any two nodes and bends at a single highest node,
combining that node's two best downward branches (LeetCode 124 generalised to an
arbitrary tree).

The traversal is iterative (explicit stack) so a deep / unbalanced tree cannot
blow the Python recursion limit — one of the edge cases flagged in the prep.

Nodes are ``0 .. n-1``; ``knots[i]`` is node ``i``'s value; ``edges`` is the
undirected edge list. Values may be negative.

Complexity: O(n).
"""

from collections import defaultdict
from typing import List, Tuple


def maximal_knot_path(knots: List[int], edges: List[Tuple[int, int]]) -> int:
    """Maximum sum over every simple path in the tree."""
    n = len(knots)
    if n == 0:
        return 0
    if n == 1:
        return knots[0]

    adj = defaultdict(list)
    for a, b in edges:
        adj[a].append(b)
        adj[b].append(a)

    # Build a parent/post-order layout from an iterative DFS rooted at 0.
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

    down = [0] * n  # best downward path sum starting at each node
    best = float("-inf")
    for u in reversed(order):  # children processed before parents
        best_child = 0
        second_child = 0
        for v in adj[u]:
            if parent[v] == u:
                gain = max(0, down[v])  # never extend into a losing branch
                if gain >= best_child:
                    second_child = best_child
                    best_child = gain
                elif gain > second_child:
                    second_child = gain
        best = max(best, knots[u] + best_child + second_child)
        down[u] = knots[u] + best_child
    return best
