from q8_array_between_min_max import values_between


def test_strictly_between_preserves_order():
    assert values_between([1, 5, 3, 9, 2, 7], 2, 7) == [5, 3]


def test_inclusive():
    assert values_between([1, 5, 3, 9, 2, 7], 2, 7, inclusive=True) == [5, 3, 2, 7]


def test_empty():
    assert values_between([], 0, 10) == []


def test_none_in_range():
    assert values_between([1, 2, 3], 10, 20) == []
