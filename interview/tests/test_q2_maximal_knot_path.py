from q2_maximal_knot_path import maximal_knot_path


def test_empty_and_single():
    assert maximal_knot_path([], []) == 0
    assert maximal_knot_path([42], []) == 42


def test_path_graph():
    # 0-1-2 with values 1,2,3 -> whole path 6.
    assert maximal_knot_path([1, 2, 3], [(0, 1), (1, 2)]) == 6


def test_all_negative_takes_single_best_node():
    assert maximal_knot_path([-1, -2, -3], [(0, 1), (1, 2)]) == -1


def test_bends_at_centre_using_two_best_branches():
    # Negative centre, positive leaves: a single leaf (7) beats the bent path.
    knots = [-10, 5, 6, 7]
    edges = [(0, 1), (0, 2), (0, 3)]
    assert maximal_knot_path(knots, edges) == 7


def test_positive_centre_combines_two_branches():
    knots = [10, 5, 6, 1]
    edges = [(0, 1), (0, 2), (0, 3)]
    # 6 + 10 + 5 = 21 (drop the weakest leaf).
    assert maximal_knot_path(knots, edges) == 21


def test_deep_chain_no_recursion_limit():
    n = 5000
    knots = [1] * n
    edges = [(i, i + 1) for i in range(n - 1)]
    assert maximal_knot_path(knots, edges) == n
