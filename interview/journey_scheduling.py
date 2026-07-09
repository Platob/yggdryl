"""Extra — Journey Scheduling (HackerRank).

Unweighted tree. Each query ``(v, k)`` starts at city ``v`` and makes ``k``
temple visits, maximising the total distance travelled. The best first leg goes
to the farthest node from ``v`` (its eccentricity); afterwards you bounce between
the two diameter endpoints, each further leg adding the diameter ``D``:

    answer = eccentricity(v) + (k - 1) * D

The eccentricity of every node is ``max(dist_to_A, dist_to_B)`` where ``A`` and
``B`` are the diameter endpoints (a standard tree fact). Three BFS passes find
them, then each query is O(1). Overall O(n + q).
"""

from collections import deque
from typing import List, Tuple


def _bfs(adj: List[List[int]], source: int, n: int) -> List[int]:
    dist = [-1] * n
    dist[source] = 0
    queue = deque([source])
    while queue:
        u = queue.popleft()
        for v in adj[u]:
            if dist[v] == -1:
                dist[v] = dist[u] + 1
                queue.append(v)
    return dist


def journey_scheduling(
    n: int, edges: List[Tuple[int, int]], queries: List[Tuple[int, int]]
) -> List[int]:
    """Answer each ``(start, temples)`` query with the maximum travel distance."""
    adj: List[List[int]] = [[] for _ in range(n)]
    for a, b in edges:
        adj[a].append(b)
        adj[b].append(a)

    # Diameter endpoints A and B via two BFS passes, then distances from each.
    far_from_0 = _bfs(adj, 0, n)
    a = far_from_0.index(max(far_from_0))
    dist_a = _bfs(adj, a, n)
    b = dist_a.index(max(dist_a))
    dist_b = _bfs(adj, b, n)
    diameter = dist_a[b]

    result = []
    for start, temples in queries:
        eccentricity = max(dist_a[start], dist_b[start])
        result.append(eccentricity + (temples - 1) * diameter)
    return result
