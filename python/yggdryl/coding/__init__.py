"""What every content coding shares: the transparent handle over a coded one.

Each codec itself is a module of its own beside this one - :mod:`yggdryl.gzip`,
:mod:`yggdryl.zlib` and :mod:`yggdryl.zstd` - as the crate gives every codec a
root file. The capitalized classes are the handles that present the decoded
bytes of a coded resource, which is what ``IOBase("app.log.gz")`` composes.
"""

from .._native import Coded, Gzip, Identity, Zlib, Zstd

__all__ = [
    "Coded",
    "Gzip",
    "Identity",
    "Zlib",
    "Zstd",
]
