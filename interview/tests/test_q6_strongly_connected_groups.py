from q6_strongly_connected_groups import (
    largest_scc_size,
    strongly_connected_components,
)


def test_three_sccs():
    n = 6
    edges = [(0, 1), (1, 2), (2, 0), (2, 3), (3, 4), (4, 3)]
    sccs = strongly_connected_components(n, edges)
    assert sorted(sccs) == [[0, 1, 2], [3, 4], [5]]
    assert largest_scc_size(n, edges) == 3


def test_dag_all_singletons():
    assert largest_scc_size(3, [(0, 1), (1, 2)]) == 1


def test_single_big_cycle():
    edges = [(0, 1), (1, 2), (2, 3), (3, 0)]
    assert largest_scc_size(4, edges) == 4


def test_no_nodes():
    assert largest_scc_size(0, []) == 0
