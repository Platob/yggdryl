"""Q6 — Strongly connected groups (SCC) of a directed graph.

Kosaraju's "two-run" algorithm (the two-pass approach the prep hints at):
1. DFS the graph, pushing nodes onto a stack in finish order.
2. DFS the transposed graph in reverse-finish order; each tree is one SCC.

Both DFS passes are iterative to stay safe on deep graphs. Nodes are
``0 .. n-1``; ``edges`` are directed ``(u, v)``. O(V + E).
"""

from typing import List, Tuple


def strongly_connected_components(
    n: int, edges: List[Tuple[int, int]]
) -> List[List[int]]:
    """Return the SCCs as lists of node labels."""
    graph = [[] for _ in range(n)]
    transpose = [[] for _ in range(n)]
    for u, v in edges:
        graph[u].append(v)
        transpose[v].append(u)

    visited = [False] * n
    finish_order: List[int] = []
    for start in range(n):
        if visited[start]:
            continue
        stack = [(start, 0)]  # (node, index of next neighbour to explore)
        while stack:
            node, i = stack[-1]
            if not visited[node]:
                visited[node] = True
            if i < len(graph[node]):
                stack[-1] = (node, i + 1)
                nxt = graph[node][i]
                if not visited[nxt]:
                    stack.append((nxt, 0))
            else:
                finish_order.append(node)
                stack.pop()

    assigned = [False] * n
    components: List[List[int]] = []
    for start in reversed(finish_order):
        if assigned[start]:
            continue
        comp: List[int] = []
        stack = [start]
        assigned[start] = True
        while stack:
            node = stack.pop()
            comp.append(node)
            for prev in transpose[node]:
                if not assigned[prev]:
                    assigned[prev] = True
                    stack.append(prev)
        components.append(sorted(comp))
    return components


def largest_scc_size(n: int, edges: List[Tuple[int, int]]) -> int:
    """Size of the largest strongly connected component (0 if no nodes)."""
    comps = strongly_connected_components(n, edges)
    return max((len(c) for c in comps), default=0)
