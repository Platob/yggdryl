from typing import Mapping

DATA_TYPE_IDS: tuple[str, ...]
DATA_TYPE_KINDS: tuple[str, ...]
TIME_UNITS: tuple[str, ...]
UNION_MODES: tuple[str, ...]
WRITE_MODES: tuple[str, ...]
CODECS: tuple[str, ...]
IO_KINDS: tuple[str, ...]
COMPATIBILITY_SCHEMES: tuple[str, ...]
LEVELS: Mapping[str, int]

__all__ = [
    "CODECS",
    "COMPATIBILITY_SCHEMES",
    "DATA_TYPE_IDS",
    "DATA_TYPE_KINDS",
    "IO_KINDS",
    "LEVELS",
    "TIME_UNITS",
    "UNION_MODES",
    "WRITE_MODES",
]
