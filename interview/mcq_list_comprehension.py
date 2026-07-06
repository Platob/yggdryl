"""MCQ prep — Python list comprehensions (merge / filter / sort / transform).

The QRT MCQ shows an input dataset and a required transformation and asks you to
pick the correct comprehension out of four options. These functions are the
canonical building blocks that show up in those options, so you can recognise
them by shape.
"""

from typing import Dict, List, Sequence, Tuple, TypeVar

T = TypeVar("T")
U = TypeVar("U")


def keep_even(nums: Sequence[int]) -> List[int]:
    """``[x for x in nums if x % 2 == 0]`` — filter."""
    return [x for x in nums if x % 2 == 0]


def squares(nums: Sequence[int]) -> List[int]:
    """``[x * x for x in nums]`` — map."""
    return [x * x for x in nums]


def filter_and_transform(nums: Sequence[int]) -> List[int]:
    """Map + filter together: double the strictly-positive values."""
    return [x * 2 for x in nums if x > 0]


def flatten(matrix: Sequence[Sequence[T]]) -> List[T]:
    """Nested comprehension — the outer ``for`` comes first."""
    return [x for row in matrix for x in row]


def merge_sum(a: Sequence[int], b: Sequence[int]) -> List[int]:
    """Element-wise merge of two equal-length sequences via ``zip``."""
    return [x + y for x, y in zip(a, b)]


def cartesian_pairs(a: Sequence[T], b: Sequence[U]) -> List[Tuple[T, U]]:
    """Every (x, y) pair — two ``for`` clauses, no ``zip``."""
    return [(x, y) for x in a for y in b]


def dict_from_lists(keys: Sequence[T], vals: Sequence[U]) -> Dict[T, U]:
    """Dict comprehension."""
    return {k: v for k, v in zip(keys, vals)}


def sorted_desc(nums: Sequence[int]) -> List[int]:
    """Not a comprehension, but the common 'sort' distractor: ``sorted(reverse=True)``."""
    return sorted(nums, reverse=True)
