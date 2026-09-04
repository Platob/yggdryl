"""The core's static enum vocabularies, and the ASCII datatypes as enum bases.

Pure enums cross the boundary as strings by convention - a datatype id is
``"int64"``, a codec is ``"gzip"`` - and this module enumerates what those
strings can be. Every tuple is unpacked from one native listing at import, so
it can never drift from the Rust constants it mirrors.

The vocabularies a caller declares are the other half: subclassing the base
:func:`fixed_ascii` builds for one width, or one of the four registered code
bases - :class:`CountryCode`, :class:`CurrencyCode`, :class:`MicCode`,
:class:`CfiCode` - names one open ASCII vocabulary whose members are the
integers their values pack into.

The four registered codes arrive already declared: :class:`Country`,
:class:`Currency`, :class:`MIC`, and :class:`CFI` are those vocabularies over
their own datatypes, open for every code they do not name.
"""

from __future__ import annotations

from typing import Mapping

from .._native import _enum_values
from .ascii import (
    AsciiCode,
    CfiCode,
    CountryCode,
    CurrencyCode,
    MicCode,
    fixed_ascii,
)
from .codes import CFI, Country, Currency, MIC

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

#: Every content coding, e.g. ``"identity"``, ``"gzip"``, ``"zstd"``.
CODECS: tuple[str, ...] = tuple(_LISTING["codecs"])

#: Every digest algorithm, e.g. ``"xxh3-64"``, ``"xxh3-128"``.
DIGEST_ALGORITHMS: tuple[str, ...] = tuple(_LISTING["digest_algorithms"])

#: Every answer a handle gives about what it addresses, e.g. ``"file"``.
IO_KINDS: tuple[str, ...] = tuple(_LISTING["io_kinds"])

#: The compatibility targets ``into_scheme_compat`` accepts, e.g. ``"arrow"``.
COMPATIBILITY_SCHEMES: tuple[str, ...] = tuple(_LISTING["compatibility_schemes"])

#: The named points of the shared 0-to-9 compression scale.
LEVELS: Mapping[str, int] = dict(_LISTING["levels"])

__all__ = [
    "AsciiCode",
    "CfiCode",
    "CountryCode",
    "CurrencyCode",
    "MicCode",
    "CFI",
    "Country",
    "Currency",
    "MIC",
    "fixed_ascii",
    "CODECS",
    "COMPATIBILITY_SCHEMES",
    "DATA_TYPE_IDS",
    "DATA_TYPE_KINDS",
    "DIGEST_ALGORITHMS",
    "IO_KINDS",
    "LEVELS",
    "TIME_UNITS",
    "UNION_MODES",
    "IO_MODES",
]
