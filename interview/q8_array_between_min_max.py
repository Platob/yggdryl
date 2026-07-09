"""Q8 — Array manipulation: values between a min and a max.

Return the elements of ``arr`` that lie between ``low`` and ``high``. Strictly
between by default; pass ``inclusive=True`` to keep the bounds. Order is
preserved. O(n).
"""

from typing import List, Sequence


def values_between(
    arr: Sequence[int], low: int, high: int, inclusive: bool = False
) -> List[int]:
    """Elements strictly (or, if ``inclusive``, loosely) between ``low`` and ``high``."""
    if inclusive:
        return [x for x in arr if low <= x <= high]
    return [x for x in arr if low < x < high]
