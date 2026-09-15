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
Writes take the ``indent`` every other codec takes: omitted is the format's
own layout, ``None`` puts the document on one line, an integer is that many
spaces per level, and ``"\\t"`` is one tab.
"""

from __future__ import annotations

from .._native import (
    xml_dumps as _dumps,
    xml_loads as loads,
    xml_loads_with_field as loads_with_field,
    xml_schema as schema,
    xml_schema_dumps as _schema_dumps,
)
from ..text._codec import _DEFAULT_INDENT, _indent_code

__all__ = [
    "dumps",
    "loads",
    "loads_with_field",
    "schema",
    "schema_dumps",
]


def dumps(
    value: object,
    name: str,
    *,
    indent: int | str | None | object = _DEFAULT_INDENT,
) -> bytes:
    """Encode one value as a whole XML document rooted at ``name``."""

    return _dumps(value, name, indent=_indent_code(indent))


def schema_dumps(
    field: object,
    *,
    indent: int | str | None | object = _DEFAULT_INDENT,
) -> bytes:
    """Write one field as the XML Schema that declares it."""

    return _schema_dumps(field, indent=_indent_code(indent))
