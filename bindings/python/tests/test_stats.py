"""Tests for the numeric-analytics reductions on the ``yggdryl.types`` numeric ``Serie``
wrappers — ``count`` / ``sum`` / ``mean`` / ``min`` / ``max`` — mirroring
``yggdryl_core::io::NumericSerie``.

``count`` is the number of **present** (non-null) elements (distinct from ``len``/``null_count``);
the reductions bridge through ``float``. ``mean``/``min``/``max`` are ``None`` over an empty or
all-null column and ``sum`` is ``0.0``; a ``NaN`` element propagates through ``sum``/``mean`` and is
skipped by ``min``/``max``.
"""

import math

from yggdryl.types import F64Serie, I32Serie


def test_int_reductions_exclude_nulls():
    col = I32Serie([1, None, 2, 6])
    # len counts every slot; count counts only present elements.
    assert len(col) == 4
    assert col.null_count == 1
    assert col.count() == 3
    assert col.sum() == 9.0
    assert col.mean() == 3.0
    assert col.min() == 1.0
    assert col.max() == 6.0


def test_int_reductions_return_float():
    col = I32Serie.from_values([2, 4])
    assert isinstance(col.sum(), float)
    assert isinstance(col.mean(), float)
    assert isinstance(col.min(), float)
    assert isinstance(col.max(), float)
    assert col.sum() == 6.0
    assert col.mean() == 3.0
    assert col.min() == 2.0
    assert col.max() == 4.0


def test_float_reductions_exclude_nulls():
    col = F64Serie([1.5, None, 2.5, -0.5])
    assert col.count() == 3
    assert col.sum() == 3.5
    assert col.mean() == 3.5 / 3
    assert col.min() == -0.5
    assert col.max() == 2.5


def test_empty_column():
    col = I32Serie()
    assert col.count() == 0
    assert col.sum() == 0.0
    assert col.mean() is None
    assert col.min() is None
    assert col.max() is None


def test_all_null_column():
    col = F64Serie([None, None])
    assert len(col) == 2
    assert col.count() == 0
    assert col.sum() == 0.0
    assert col.mean() is None
    assert col.min() is None
    assert col.max() is None


def test_float_nan_propagates_through_sum_and_mean():
    col = F64Serie([1.0, float("nan"), 3.0])
    assert col.count() == 3
    assert math.isnan(col.sum())
    assert math.isnan(col.mean())


def test_float_nan_skipped_by_min_max():
    col = F64Serie([2.0, float("nan"), 5.0, 1.0])
    assert col.min() == 1.0
    assert col.max() == 5.0


def test_float_all_nan_min_max():
    # Every present element is NaN: min/max reduce over NaNs (f64::min/max skips NaN, but with
    # only NaNs the result stays NaN), while count still sees them as present.
    col = F64Serie([float("nan"), float("nan")])
    assert col.count() == 2
    assert math.isnan(col.sum())
    assert math.isnan(col.min())
    assert math.isnan(col.max())
