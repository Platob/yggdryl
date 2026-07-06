from journey_scheduling import journey_scheduling


def test_path_tree():
    # 0-1-2-3-4, diameter 4.
    edges = [(0, 1), (1, 2), (2, 3), (3, 4)]
    queries = [(2, 1), (0, 1), (0, 3)]
    assert journey_scheduling(5, edges, queries) == [2, 4, 12]


def test_star_tree():
    # centre 0 with leaves 1,2,3; diameter 2.
    edges = [(0, 1), (0, 2), (0, 3)]
    queries = [(0, 1), (1, 1), (1, 2)]
    assert journey_scheduling(4, edges, queries) == [1, 2, 4]


def test_single_node():
    assert journey_scheduling(1, [], [(0, 1), (0, 5)]) == [0, 0]
