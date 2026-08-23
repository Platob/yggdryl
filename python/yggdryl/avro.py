"""Apache Avro schemas and Scalar codecs backed entirely by the Rust core.

The object-container pair works on bytes and returns the writer schema,
metadata, and natural Python rows. A reader schema applies Avro's native
resolution rules while decoding. The single-object pair carries the standard
schema fingerprint framing used by message systems.

Schema and decode entry points accept ``max_depth``, ``max_input_bytes``, and
``max_nodes`` so untrusted inputs use the Rust core's decoding budget.
"""

from __future__ import annotations

from ._native import (
    AvroBlock as Block,
    AvroBlockIterator as BlockIterator,
    AvroContainer as Container,
    AvroSchema as Schema,
    avro_dumps as dumps,
    avro_dumps_single as dumps_single,
    avro_blocks as blocks,
    avro_loads as loads,
    avro_loads_single as loads_single,
)

__all__ = [
    "Block",
    "BlockIterator",
    "Container",
    "Schema",
    "dumps",
    "dumps_single",
    "blocks",
    "loads",
    "loads_single",
]
