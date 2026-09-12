from typing import Mapping

from .string import (
    AsciiCode as AsciiCode,
    CfiCode as CfiCode,
    CountryCode as CountryCode,
    CurrencyCode as CurrencyCode,
    MicCode as MicCode,
    fixed_ascii as fixed_ascii,
)
from .codes import (
    CFI as CFI,
    Country as Country,
    Currency as Currency,
    MIC as MIC,
)

DATA_TYPE_IDS: tuple[str, ...]
DATA_TYPE_KINDS: tuple[str, ...]
TIME_UNITS: tuple[str, ...]
UNION_MODES: tuple[str, ...]
IO_MODES: tuple[str, ...]
IO_WRITE_MODES: tuple[str, ...]
LEADING_FRAGMENTS: tuple[str, ...]
EDGE_ALGORITHMS: tuple[str, ...]
FORMATS: tuple[str, ...]
CHARSETS: tuple[str, ...]
CODECS: tuple[str, ...]
DIGEST_ALGORITHMS: tuple[str, ...]
IO_KINDS: tuple[str, ...]
PYTHON_KINDS: tuple[str, ...]
COMPATIBILITY_SCHEMES: tuple[str, ...]
NULLABILITIES: tuple[str, ...]
REPRESENTATIONS: tuple[str, ...]
LEVELS: Mapping[str, int]

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
    "CHARSETS",
    "CODECS",
    "COMPATIBILITY_SCHEMES",
    "PYTHON_KINDS",
    "DATA_TYPE_IDS",
    "DATA_TYPE_KINDS",
    "DIGEST_ALGORITHMS",
    "IO_KINDS",
    "LEVELS",
    "NULLABILITIES",
    "REPRESENTATIONS",
    "TIME_UNITS",
    "UNION_MODES",
    "IO_MODES",
    "IO_WRITE_MODES",
    "EDGE_ALGORITHMS",
    "FORMATS",
    "LEADING_FRAGMENTS",
]
