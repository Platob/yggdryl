"""The plain-text medium, and the machinery its structured codecs share.

:mod:`~yggdryl.text.codec` is the format-directed facade the four structured
schemes ride on; each scheme is a package of its own beside this one -
:mod:`yggdryl.json`, :mod:`yggdryl.yaml`, :mod:`yggdryl.toml` and
:mod:`yggdryl.xml`. The line values are here because a physical line is what
this medium reads.
"""

from .._native import (
    TextEntries,
    TextEntry,
    TextLine,
    TextLines,
    TextOptions,
)
from . import codec

__all__ = [
    "TextEntries",
    "TextEntry",
    "TextLine",
    "TextLines",
    "TextOptions",
    "codec",
]
