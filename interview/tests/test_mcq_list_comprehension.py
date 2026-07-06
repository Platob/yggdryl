from mcq_list_comprehension import (
    cartesian_pairs,
    dict_from_lists,
    filter_and_transform,
    flatten,
    keep_even,
    merge_sum,
    sorted_desc,
    squares,
)


def test_keep_even():
    assert keep_even([1, 2, 3, 4, 5, 6]) == [2, 4, 6]
    assert keep_even([]) == []


def test_squares():
    assert squares([1, 2, 3]) == [1, 4, 9]


def test_filter_and_transform():
    assert filter_and_transform([-2, -1, 0, 1, 2]) == [2, 4]


def test_flatten():
    assert flatten([[1, 2], [3], [4, 5, 6]]) == [1, 2, 3, 4, 5, 6]
    assert flatten([]) == []


def test_merge_sum():
    assert merge_sum([1, 2, 3], [10, 20, 30]) == [11, 22, 33]


def test_cartesian_pairs():
    assert cartesian_pairs([1, 2], ["a", "b"]) == [(1, "a"), (1, "b"), (2, "a"), (2, "b")]


def test_dict_from_lists():
    assert dict_from_lists(["a", "b"], [1, 2]) == {"a": 1, "b": 2}


def test_sorted_desc():
    assert sorted_desc([3, 1, 2]) == [3, 2, 1]
