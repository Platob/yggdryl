"""Extra — Friend Circle Queries (HackerRank).

Each query introduces a friendship between two people (identified by arbitrary
integer ids). After each query, report the size of the largest friend circle so
far. Union-find with path compression and union by size; ids are added lazily,
so the structure only ever holds people who have appeared. O(q α).
"""

from typing import Dict, List, Tuple


def friend_circle_queries(queries: List[Tuple[int, int]]) -> List[int]:
    """Largest circle size after each ``(a, b)`` friendship query."""
    parent: Dict[int, int] = {}
    size: Dict[int, int] = {}
    largest = 0
    result: List[int] = []

    def find(x: int) -> int:
        parent.setdefault(x, x)
        size.setdefault(x, 1)
        root = x
        while parent[root] != root:
            root = parent[root]
        while parent[x] != root:
            parent[x], x = root, parent[x]
        return root

    for a, b in queries:
        ra, rb = find(a), find(b)
        if ra != rb:
            if size[ra] < size[rb]:
                ra, rb = rb, ra
            parent[rb] = ra
            size[ra] += size[rb]
            largest = max(largest, size[ra])
        else:
            largest = max(largest, size[ra])
        result.append(largest)
    return result
