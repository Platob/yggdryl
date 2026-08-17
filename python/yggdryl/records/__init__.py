"""Dataclass-compatible records with native Yggdryl schemas.

Use :func:`record` anywhere :func:`dataclasses.dataclass` would be used. The
decorated class remains a standard dataclass and gains cached native schema,
``from_dict``, and ``to_dict`` conveniences.
"""

from dataclasses import (
    KW_ONLY,
    MISSING,
    Field,
    FrozenInstanceError,
    InitVar,
    asdict,
    astuple,
    dataclass,
    field,
    fields,
    is_dataclass,
    make_dataclass,
    replace,
)
from typing import TypeAlias

from .._codec import CodecFormat
from .._native import Field as SchemaField
from ._hints import datatype_from_pyhint, field_from_pyhint
from ._records import Record, from_dict, record, schema_field, schema_fields, to_dict

Format: TypeAlias = CodecFormat

__all__ = [
    "Field",
    "FrozenInstanceError",
    "Format",
    "InitVar",
    "KW_ONLY",
    "MISSING",
    "Record",
    "SchemaField",
    "asdict",
    "astuple",
    "dataclass",
    "datatype_from_pyhint",
    "field",
    "field_from_pyhint",
    "fields",
    "from_dict",
    "is_dataclass",
    "make_dataclass",
    "record",
    "replace",
    "schema_field",
    "schema_fields",
    "to_dict",
]
