from friend_circle_queries import friend_circle_queries


def test_growing_single_circle():
    assert friend_circle_queries([(1, 2), (1, 3), (1, 4)]) == [2, 3, 4]


def test_two_circles_then_merge():
    assert friend_circle_queries([(1, 2), (3, 4), (2, 3)]) == [2, 2, 4]


def test_redundant_query_keeps_max():
    assert friend_circle_queries([(1, 2), (1, 2)]) == [2, 2]


def test_empty():
    assert friend_circle_queries([]) == []
