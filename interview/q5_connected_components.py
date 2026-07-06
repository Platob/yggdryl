"""Q5 — Graph: connected components, min/max value per component.

Undirected graph on nodes ``0 .. n-1``. Find the connected components; for each,
report the smallest and largest node label and their difference. Uses a
union-find (disjoint set) with path compression and union by size — O(n + E α).
"""

from typing import Dict, List, Tuple


class _DSU:
    def __init__(self, n: int) -> None:
        self.parent = list(range(n))
        self.size = [1] * n

    def find(self, x: int) -> int:
        root = x
        while self.parent[root] != root:
            root = self.parent[root]
        while self.parent[x] != root:  # path compression
            self.parent[x], x = root, self.parent[x]
        return root

    def union(self, a: int, b: int) -> None:
        ra, rb = self.find(a), self.find(b)
        if ra == rb:
            return
        if self.size[ra] < self.size[rb]:
            ra, rb = rb, ra
        self.parent[rb] = ra
        self.size[ra] += self.size[rb]


def connected_components(n: int, edges: List[Tuple[int, int]]) -> List[List[int]]:
    """Return the components as sorted lists of node labels."""
    dsu = _DSU(n)
    for a, b in edges:
        dsu.union(a, b)
    groups: Dict[int, List[int]] = {}
    for node in range(n):
        groups.setdefault(dsu.find(node), []).append(node)
    return [sorted(members) for members in groups.values()]


def component_min_max_diffs(
    n: int, edges: List[Tuple[int, int]]
) -> List[Tuple[int, int, int]]:
    """For each component return ``(min_label, max_label, max - min)``."""
    result = []
    for comp in connected_components(n, edges):
        lo, hi = comp[0], comp[-1]
        result.append((lo, hi, hi - lo))
    return result
