from q2b_cut_the_tree import cut_the_tree


def test_hackerrank_sample():
    # HackerRank sample (converted to 0-indexed); answer 400.
    values = [100, 200, 100, 500, 100, 600]
    edges = [(0, 1), (1, 2), (1, 4), (3, 4), (4, 5)]
    assert cut_the_tree(values, edges) == 400


def test_two_nodes():
    assert cut_the_tree([1, 10], [(0, 1)]) == 9


def test_single_node_has_no_cut():
    assert cut_the_tree([5], []) == 0


def test_balanced_even_split():
    values = [1, 1, 1, 1]
    edges = [(0, 1), (1, 2), (2, 3)]
    # Cut the middle edge -> two halves of sum 2 each -> difference 0.
    assert cut_the_tree(values, edges) == 0
