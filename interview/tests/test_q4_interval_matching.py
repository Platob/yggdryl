from q4_interval_matching import (
    count_intervals_containing,
    count_numbers_in_any_interval,
    numbers_in_any_interval,
)


def test_count_numbers_in_any_interval():
    numbers = [1, 2, 3, 4, 5, 10]
    intervals = [(2, 4), (9, 12)]
    assert count_numbers_in_any_interval(numbers, intervals) == 4


def test_numbers_in_any_interval_values():
    numbers = [1, 2, 3, 4, 5, 10]
    intervals = [(2, 4), (9, 12)]
    assert numbers_in_any_interval(numbers, intervals) == [2, 3, 4, 10]


def test_overlapping_intervals_are_merged():
    numbers = [0, 1, 2, 3, 4, 5, 6]
    intervals = [(1, 3), (2, 5)]
    assert count_numbers_in_any_interval(numbers, intervals) == 5  # 1..5


def test_empty_intervals():
    assert count_numbers_in_any_interval([1, 2, 3], []) == 0


def test_count_intervals_containing():
    intervals = [(1, 5), (2, 4), (10, 12)]
    assert count_intervals_containing(3, intervals) == 2
    assert count_intervals_containing(11, intervals) == 1
    assert count_intervals_containing(7, intervals) == 0
