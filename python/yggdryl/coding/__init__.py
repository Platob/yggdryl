"""Content codings over bytes and storage handles.

The lowercase modules are the whole-buffer codecs; the capitalized classes are
the handles that present the decoded bytes of a coded resource, which is what
``IOBase("app.log.gz")`` composes.
"""

from .._native import Coded, Gzip, Identity, Zlib, Zstd
from . import gzip, zlib, zstd

__all__ = [
    "Coded",
    "Gzip",
    "Identity",
    "Zlib",
    "Zstd",
    "gzip",
    "zlib",
    "zstd",
]
