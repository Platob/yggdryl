from q5_connected_components import (
    component_min_max_diffs,
    connected_components,
)


def test_components():
    comps = connected_components(6, [(0, 1), (1, 2), (3, 4)])
    assert sorted(comps) == [[0, 1, 2], [3, 4], [5]]


def test_min_max_diffs():
    result = component_min_max_diffs(6, [(0, 1), (1, 2), (3, 4)])
    assert sorted(result) == [(0, 2, 2), (3, 4, 1), (5, 5, 0)]


def test_no_edges_all_singletons():
    assert component_min_max_diffs(3, []) == [(0, 0, 0), (1, 1, 0), (2, 2, 0)]


def test_fully_connected():
    result = component_min_max_diffs(4, [(0, 1), (1, 2), (2, 3)])
    assert result == [(0, 3, 3)]
