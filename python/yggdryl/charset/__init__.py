"""Character encodings over bytes.

``decode`` and ``encode`` answer what ``bytes.decode(name)`` and
``str.encode(name)`` answer over the same names; what they add is the crate's
own refusal - the charset, the byte position, and the byte or scalar found
there - and one vocabulary shared with the media types, handles, and record
options that already name a charset.

A whole resource is read by naming the charset on its media type rather than
by calling these: a handle whose media type says ``charset=windows-1252``
reads as text without a caller passing anything.
"""

from __future__ import annotations

from .._native import (
    charset_bom,
    charset_canonical_name,
    charset_decode,
    charset_decode_lossy,
    charset_encode,
    charset_from_bom,
)
from ..enums import CHARSETS

__all__ = [
    "CHARSETS",
    "bom",
    "canonical_name",
    "decode",
    "decode_lossy",
    "encode",
    "from_bom",
]

decode = charset_decode
decode_lossy = charset_decode_lossy
encode = charset_encode
from_bom = charset_from_bom
bom = charset_bom
canonical_name = charset_canonical_name
