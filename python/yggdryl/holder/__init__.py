"""Byte-storage holders: the handle contract and the role each one takes.

``IOBase`` is the contract; every other name here is the class one core
implementation reports itself as, so ``type(handle)`` says which storage is
doing the work. Construction picks the role - and the coding and record
implementation a name declares - so these are the explicit spellings, for a
caller who wants a particular one.
"""

from .._native import (
    Buffer,
    Buffered,
    FsFile,
    FsFolder,
    FsPath,
    IOBase,
    IOCursor,
    LocalFile,
    LocalFolder,
    LocalPath,
    S3File,
    S3Folder,
    S3Path,
)

__all__ = [
    "Buffer",
    "Buffered",
    "FsFile",
    "FsFolder",
    "FsPath",
    "IOBase",
    "IOCursor",
    "LocalFile",
    "LocalFolder",
    "LocalPath",
    "S3File",
    "S3Folder",
    "S3Path",
]
