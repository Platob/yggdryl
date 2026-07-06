"""Q9 — Mathematical formula to algorithm ("LEM").

The exact "LEM" formula was not given in the prep, so this module demonstrates
the *technique* the question tests: take a closed-form mathematical formula,
generalise it, and turn it into a working (iterative) algorithm — then prove the
two agree.

Worked example: the sum of the first ``n`` squares.
    closed form : n (n + 1) (2n + 1) / 6
    algorithm   : accumulate 1**2 + 2**2 + ... + n**2

``weighted_power_sum`` is the generalisation (sum of ``coefficient * i**power``),
which the closed forms are special cases of. Adapt these to the real formula on
the day.
"""

from typing import Sequence


def sum_of_squares_formula(n: int) -> int:
    """Closed form: n(n+1)(2n+1)/6."""
    if n < 0:
        raise ValueError(f"expected n >= 0, got {n}")
    return n * (n + 1) * (2 * n + 1) // 6


def sum_of_squares_algorithm(n: int) -> int:
    """Iterative equivalent of :func:`sum_of_squares_formula`."""
    if n < 0:
        raise ValueError(f"expected n >= 0, got {n}")
    return sum(i * i for i in range(1, n + 1))


def sum_first_n_formula(n: int) -> int:
    """Closed form: n(n+1)/2."""
    if n < 0:
        raise ValueError(f"expected n >= 0, got {n}")
    return n * (n + 1) // 2


def weighted_power_sum(coefficients: Sequence[int], power: int) -> int:
    """Generalised series: sum of ``coefficients[i] * (i + 1) ** power``.

    A template for turning an indexed formula into an algorithm.
    """
    return sum(c * (i + 1) ** power for i, c in enumerate(coefficients))
