from q3_segment_intersections import (
    max_overlap,
    merge_intervals,
    total_covered_length,
)


def test_merge_basic():
    assert merge_intervals([(1, 3), (2, 6), (8, 10), (15, 18)]) == [
        (1, 6),
        (8, 10),
        (15, 18),
    ]


def test_merge_touching_endpoints():
    assert merge_intervals([(1, 3), (3, 5)]) == [(1, 5)]


def test_merge_empty():
    assert merge_intervals([]) == []


def test_total_covered_length():
    assert total_covered_length([(1, 3), (2, 6), (8, 10)]) == 7  # (1,6)=5 + (8,10)=2


def test_max_overlap():
    assert max_overlap([(3, 5), (4, 6)]) == 2
    assert max_overlap([(1, 4), (2, 3), (3, 5)]) == 3  # point 3 in all three
    assert max_overlap([]) == 0
    assert max_overlap([(1, 2), (3, 4)]) == 1
