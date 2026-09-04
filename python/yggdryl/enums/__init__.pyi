from typing import Mapping

from .ascii import (
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
CODECS: tuple[str, ...]
DIGEST_ALGORITHMS: tuple[str, ...]
IO_KINDS: tuple[str, ...]
COMPATIBILITY_SCHEMES: tuple[str, ...]
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
