"""The one boundary between foreign columnar objects and native Arrow values."""

from ._native import ArrowValue, arrow_shapes

#: Every shape an :class:`ArrowValue` can hold, in widening order.
SHAPES: tuple[str, ...] = tuple(arrow_shapes())

__all__ = ["SHAPES", "ArrowValue"]
