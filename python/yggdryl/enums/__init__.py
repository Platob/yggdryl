"""The core's static enum vocabularies, and the ASCII datatypes as enum bases.

Pure enums cross the boundary as strings by convention - a datatype id is
``"int64"``, a codec is ``"gzip"`` - and this module enumerates what those
strings can be. Every tuple is unpacked from one native listing at import, so
it can never drift from the Rust constants it mirrors.

The vocabularies a caller declares are the other half: subclassing the base
:func:`fixed_ascii` builds for one width, or one of the four registered code
bases - :class:`CountryCode`, :class:`CcyCode`, :class:`MicCode`,
:class:`CfiCode` - names one open ASCII vocabulary whose members are the
integers their values pack into.

The four registered codes arrive already declared: :class:`Country`,
:class:`Ccy`, :class:`MIC`, and :class:`CFI` are those vocabularies over
their own datatypes, open for every code they do not name.
"""

from __future__ import annotations

from typing import Mapping

from .._native import _enum_values
from .string import (
    AsciiCode,
    CfiCode,
    CountryCode,
    CcyCode,
    MicCode,
    fixed_ascii,
)
from .codes import CFI, Ccy, Country, MIC

_LISTING = _enum_values()

#: Every datatype variant identity, e.g. ``"int64"``, ``"decimal128"``.
DATA_TYPE_IDS: tuple[str, ...] = tuple(_LISTING["data_type_ids"])

#: Every datatype family, e.g. ``"integer"``, ``"decimal"``.
DATA_TYPE_KINDS: tuple[str, ...] = tuple(_LISTING["data_type_kinds"])

#: Every temporal resolution and interval layout, e.g. ``"ms"``, ``"year_month"``.
TIME_UNITS: tuple[str, ...] = tuple(_LISTING["time_units"])

#: Both union modes: ``"sparse"`` and ``"dense"``.
UNION_MODES: tuple[str, ...] = tuple(_LISTING["union_modes"])

#: Every generic I/O intent.
IO_MODES: tuple[str, ...] = tuple(_LISTING["io_modes"])

#: The subset of :data:`IO_MODES` a record write accepts.
IO_WRITE_MODES: tuple[str, ...] = tuple(_LISTING["io_write_modes"])

#: What a text read does with a first line that is only part of a record.
LEADING_FRAGMENTS: tuple[str, ...] = tuple(_LISTING["leading_fragments"])

#: Every geography edge-interpolation algorithm.
EDGE_ALGORITHMS: tuple[str, ...] = tuple(_LISTING["edge_algorithms"])

#: Every structured text format, e.g. ``"json"``, ``"yaml"``.
FORMATS: tuple[str, ...] = tuple(_LISTING["formats"])

#: Every content coding, e.g. ``"identity"``, ``"gzip"``, ``"zstd"``.
CODECS: tuple[str, ...] = tuple(_LISTING["codecs"])

#: Every character encoding, e.g. ``"utf-8"``, ``"windows-1252"``.
CHARSETS: tuple[str, ...] = tuple(_LISTING["charsets"])

#: Every digest algorithm, e.g. ``"xxh3-64"``, ``"xxh3-128"``.
DIGEST_ALGORITHMS: tuple[str, ...] = tuple(_LISTING["digest_algorithms"])

#: Every answer a handle gives about what it addresses, e.g. ``"file"``.
IO_KINDS: tuple[str, ...] = tuple(_LISTING["io_kinds"])

#: Every Python form a `PYTHON:kind` declaration names, e.g. ``"dataclass"``.
PYTHON_KINDS: tuple[str, ...] = tuple(_LISTING["python_kinds"])

#: The compatibility targets ``into_scheme_compat`` accepts, e.g. ``"arrow"``.
COMPATIBILITY_SCHEMES: tuple[str, ...] = tuple(_LISTING["compatibility_schemes"])

#: What a cast makes a same-width pair carry.
REPRESENTATIONS: tuple[str, ...] = tuple(_LISTING["representations"])

#: The named points of the shared 0-to-9 compression scale.
LEVELS: Mapping[str, int] = dict(_LISTING["levels"])

#: Every leaf kind a ``graph.MarketData`` may hold, e.g. ``"order_event"``,
#: ``"book_side"``: the ``MarketData.kinds`` spellings, in declaration order.
MARKET_KINDS: tuple[str, ...] = tuple(_LISTING["market_kinds"])

#: Every market-data update action a book entry states, FIX's own codes
#: beside ``"snapshot"``.
MD_UPDATE_ACTIONS: tuple[str, ...] = tuple(_LISTING["md_update_actions"])

#: The sixteen columns every graph event is stated in, in schema order.
EVENT_COLUMNS: tuple[str, ...] = tuple(_LISTING["event_columns"])

#: The nineteen columns a market element states, in schema order.
MARKET_COLUMNS: tuple[str, ...] = tuple(_LISTING["market_columns"])

#: The eight columns a market operation states, in schema order.
OPERATION_COLUMNS: tuple[str, ...] = tuple(_LISTING["operation_columns"])

#: Every named reading of a ``marketdata`` stream ``graph.MarketData.plan``
#: and ``apply_view`` take, e.g. ``"orders"``, ``"book_sides"``, in
#: declaration order.
MARKET_VIEWS: tuple[str, ...] = tuple(_LISTING["market_views"])

__all__ = [
    "AsciiCode",
    "CfiCode",
    "CountryCode",
    "CcyCode",
    "MicCode",
    "CFI",
    "Country",
    "Ccy",
    "MIC",
    "fixed_ascii",
    "CHARSETS",
    "CODECS",
    "COMPATIBILITY_SCHEMES",
    "PYTHON_KINDS",
    "DATA_TYPE_IDS",
    "DATA_TYPE_KINDS",
    "DIGEST_ALGORITHMS",
    "EVENT_COLUMNS",
    "IO_KINDS",
    "LEVELS",
    "MARKET_COLUMNS",
    "MARKET_KINDS",
    "MARKET_VIEWS",
    "MD_UPDATE_ACTIONS",
    "OPERATION_COLUMNS",
    "REPRESENTATIONS",
    "TIME_UNITS",
    "UNION_MODES",
    "IO_MODES",
    "IO_WRITE_MODES",
    "EDGE_ALGORITHMS",
    "FORMATS",
    "LEADING_FRAGMENTS",
]
