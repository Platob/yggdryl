"""XML documents and XML Schema, backed entirely by the Rust core.

``loads`` and ``dumps`` are the value pair: one document in, one natural
Python value out, and back. Every leaf a document carries is text, because
text is all a document proves - ``loads_with_field`` is what types them,
through the same value contract every other codec uses.

``schema`` reads an XML Schema as the ``Field`` it declares, and
``schema_dumps`` writes one back. The field is what a record read already
takes, so a schema types a document through the ordinary declared-field path
rather than through a surface of its own.

Decode entry points accept ``max_depth``, ``max_input_bytes`` and
``max_nodes`` so an untrusted document uses the core's decoding budget.
"""

from __future__ import annotations

from .._native import (
    xml_dumps as dumps,
    xml_loads as loads,
    xml_loads_with_field as loads_with_field,
    xml_schema as schema,
    xml_schema_dumps as schema_dumps,
)

__all__ = [
    "dumps",
    "loads",
    "loads_with_field",
    "schema",
    "schema_dumps",
]
