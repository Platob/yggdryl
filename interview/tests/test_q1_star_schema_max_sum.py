from q1_star_schema_max_sum import max_star_sum


def test_leetcode_example_1():
    vals = [1, 2, 3, 4, 10, -10, -20]
    edges = [[0, 1], [1, 2], [1, 3], [3, 4], [3, 5], [3, 6]]
    # Centre node 3 (value 4) + best two neighbours 10 and 2 -> 16.
    assert max_star_sum(vals, edges, 2) == 16


def test_leetcode_example_2_single_node():
    assert max_star_sum([-5], [], 0) == -5


def test_all_negative_neighbours_prefers_lone_centre():
    vals = [3, -1, -2]
    edges = [[0, 1], [0, 2]]
    assert max_star_sum(vals, edges, 2) == 3


def test_k_zero_ignores_neighbours():
    vals = [1, 100]
    edges = [[0, 1]]
    assert max_star_sum(vals, edges, 0) == 100


def test_k_limits_neighbours():
    vals = [0, 5, 4, 3]
    edges = [[0, 1], [0, 2], [0, 3]]
    assert max_star_sum(vals, edges, 1) == 5
    assert max_star_sum(vals, edges, 2) == 9
    assert max_star_sum(vals, edges, 3) == 12
