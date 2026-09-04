from typing import Mapping

from .ascii import (
    Ascii16 as Ascii16,
    Ascii24 as Ascii24,
    Ascii32 as Ascii32,
    Ascii64 as Ascii64,
    Ascii96 as Ascii96,
    Ascii128 as Ascii128,
    AsciiCode as AsciiCode,
)

DATA_TYPE_IDS: tuple[str, ...]
DATA_TYPE_KINDS: tuple[str, ...]
TIME_UNITS: tuple[str, ...]
UNION_MODES: tuple[str, ...]
IO_MODES: tuple[str, ...]
CODECS: tuple[str, ...]
IO_KINDS: tuple[str, ...]
COMPATIBILITY_SCHEMES: tuple[str, ...]
LEVELS: Mapping[str, int]

__all__ = [
    "Ascii16",
    "Ascii24",
    "Ascii32",
    "Ascii64",
    "Ascii96",
    "Ascii128",
    "AsciiCode",
    "CODECS",
    "COMPATIBILITY_SCHEMES",
    "DATA_TYPE_IDS",
    "DATA_TYPE_KINDS",
    "IO_KINDS",
    "LEVELS",
    "TIME_UNITS",
    "UNION_MODES",
    "IO_MODES",
]
