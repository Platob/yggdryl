"""Native ``Serie``: many values, as a schema-free run or as the Arrow buffers of one field.

``SerieReader`` is the same over a stream: one record ``Serie`` per batch,
each cast by one plan.
"""

from ._native import (
    FixedSizeListSerie,
    LargeListSerie,
    LargeListViewSerie,
    ListSerie,
    ListViewSerie,
    MapSerie,
    Serie,
    SerieReader,
    StructSerie,
)

__all__ = [
    "FixedSizeListSerie",
    "LargeListSerie",
    "LargeListViewSerie",
    "ListSerie",
    "ListViewSerie",
    "MapSerie",
    "Serie",
    "SerieReader",
    "StructSerie",
]
