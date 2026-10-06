"""The pyarrow extension types of the datatypes Arrow cannot state alone.

Every `yggdryl.*` name the core writes - the seventeen codes, the six enum
leaves, the fixed decimals, the string and bytes documents, the version, URL,
URN, timezone, MIME and media types - is registered with pyarrow when
`yggdryl` is imported, from the one list the core keeps
(`DataType.ARROW_EXTENSION_NAMES`). A column then crosses into pyarrow as the
extension type it is, and a pyarrow extension array, type or scalar crosses
back as the datatype it names. The canonical `arrow.uuid` is pyarrow's own;
`geoarrow.wkb` and `arrow.parquet.variant` belong to their own ecosystems and
cross as field metadata wherever nothing registers them.

A type holds the document the core wrote beside its name, as bytes, and
reads nothing in it: `datatype` asks the core what the name and the document
say about the storage. An array of one reads its values as the core reads
the column, in one pass.

pyarrow before 21 exports an extension array laid out over a view - the
`ascii_view`, `large_utf8_view` and `large_binary_view` leaves and their
kin - without its variadic buffers, through the C Data Interface and through
its own casts alike. On those releases such a column crosses into `yggdryl`
as its storage, the extension in the field's metadata (`_exportable`), and
one nested below a column's top level is refused by name.
"""

from __future__ import annotations

from typing import Any

import pyarrow as pa  # type: ignore[import-untyped]

from ._native import DataType, Scalar, Serie

#: The prefix of the names this module registers.
PREFIX = "yggdryl."


class YggdrylType(pa.ExtensionType):  # type: ignore[misc]
    """One `yggdryl.*` extension type over the storage the core lays its
    datatype out in, carrying the core's document as it was written."""

    #: The extension name each registered subclass stands for.
    _extension_name = ""

    def __init__(self, storage_type: pa.DataType, document: bytes = b"") -> None:
        self._document = bytes(document)
        super().__init__(storage_type, self._extension_name)

    def __arrow_ext_serialize__(self) -> bytes:
        return self._document

    @classmethod
    def __arrow_ext_deserialize__(
        cls, storage_type: pa.DataType, serialized: bytes
    ) -> YggdrylType:
        return cls(storage_type, serialized)

    def __arrow_ext_scalar_class__(self) -> type[pa.ExtensionScalar]:
        return YggdrylScalar

    def __arrow_ext_class__(self) -> type[pa.ExtensionArray]:
        return YggdrylArray

    def __reduce__(self) -> tuple[Any, ...]:
        return extension_type, (self.extension_name, self.storage_type, self._document)

    # pyarrow compares an extension type by its name and storage alone, and
    # two leaves of one storage - `sized_ascii(4)`, `sized_ascii(8)` - differ
    # only in the document.
    def __eq__(self, other: object) -> bool:
        if isinstance(other, YggdrylType):
            return (self.extension_name, self.storage_type, self._document) == (
                other.extension_name,
                other.storage_type,
                other._document,
            )
        return NotImplemented

    # pyarrow's own `__ne__` would compare the name and the storage alone, so
    # the inverse of the equality above is spelled too.
    def __ne__(self, other: object) -> bool:
        equal = self.__eq__(other)
        return equal if equal is NotImplemented else not equal

    def __hash__(self) -> int:
        return hash((self.extension_name, self.storage_type, self._document))

    # pyarrow renders a Python extension type as `extension<name<Class>>`; its
    # own canonical types print `extension<arrow.uuid>`, a parameterized one
    # its parameters in brackets. The document is what tells two leaves of one
    # storage apart, as equality does, so it prints as the datatype it states.
    # A pyarrow field or schema still renders through Arrow C++.
    def __str__(self) -> str:
        if not self._document:
            return f"extension<{self.extension_name}>"
        return f"extension<{self.extension_name}[{self.datatype}]>"

    def __repr__(self) -> str:
        return f"YggdrylType({self})"

    @property
    def document(self) -> bytes:
        """The `ARROW:extension:metadata` document beside the name."""
        return self._document

    @property
    def datatype(self) -> DataType:
        """The datatype the name and the document state over the storage,
        read by the core."""
        return DataType.from_arrow(self)


class YggdrylScalar(pa.ExtensionScalar):  # type: ignore[misc]
    """One value of a `yggdryl.*` extension type, read as the core reads it:
    a code as its text, an enum member as its member, a decimal at its
    scale."""

    def as_py(self, *args: Any, **kwargs: Any) -> Any:
        return Scalar.from_(self).as_py()


class YggdrylArray(pa.ExtensionArray):  # type: ignore[misc]
    """A column of a `yggdryl.*` extension type, read as the core reads it:
    `to_pylist` lands the column once rather than each value apart."""

    def to_pylist(self, *args: Any, **kwargs: Any) -> list[Any]:
        values: list[Any] = Serie.from_arrow_array(self).as_py()
        return values


#: Each registered name's class.
_TYPES: dict[str, type[YggdrylType]] = {}


def extension_type(
    name: str, storage_type: pa.DataType, document: bytes = b""
) -> YggdrylType:
    """The registered extension type `name` over `storage_type`, carrying
    `document`; a name `yggdryl` does not register is a `KeyError`."""
    try:
        registered = _TYPES[name]
    except KeyError:
        raise KeyError(f"expected a yggdryl extension name, got {name!r}") from None
    return registered(storage_type, document)


#: Whether this pyarrow exports an extension array over a view layout
#: without its variadic buffers: every release before 21.
_VIEW_EXPORT_BROKEN = int(pa.__version__.split(".")[0]) < 21


def _is_view_extension(dtype: pa.DataType) -> bool:
    return isinstance(dtype, pa.BaseExtensionType) and (
        pa.types.is_string_view(dtype.storage_type) or pa.types.is_binary_view(dtype.storage_type)
    )


def _holds_view_extension(dtype: pa.DataType) -> bool:
    """Whether `dtype`, or a type below it, is an extension over a view."""
    if _is_view_extension(dtype):
        return True
    if isinstance(dtype, pa.BaseExtensionType):
        return _holds_view_extension(dtype.storage_type)
    if pa.types.is_dictionary(dtype) or pa.types.is_run_end_encoded(dtype):
        return _holds_view_extension(dtype.value_type)
    return any(_holds_view_extension(dtype.field(at).type) for at in range(dtype.num_fields))


def _storage_field(field: pa.Field) -> pa.Field:
    """`field`'s extension laid out as its storage, the extension in the
    field's metadata, which is how the C Data Interface states one."""
    dtype = field.type
    metadata = dict(field.metadata or {})
    metadata[b"ARROW:extension:name"] = dtype.extension_name.encode()
    metadata[b"ARROW:extension:metadata"] = dtype.__arrow_ext_serialize__()
    return pa.field(field.name, dtype.storage_type, field.nullable, metadata)


def _refusal(name: str, dtype: pa.DataType) -> ValueError:
    return ValueError(
        f"expected an extension over a view layout only at a column's top level on pyarrow "
        f"{pa.__version__}, which exports one nested deeper without its buffers, got {name}: "
        f"{dtype}; pyarrow 21 and later export it"
    )


def _exportable(value: Any) -> Any:
    """`value` as this pyarrow's C Data Interface exports it intact: on
    pyarrow before 21, a batch, a table or a stream whose columns are
    extensions over a view crosses with each as its storage, the extension in
    the field's metadata, and one nested below a column's top level is
    refused by name. Every other value crosses as it is."""
    if not _VIEW_EXPORT_BROKEN:
        return value
    if isinstance(value, pa.Array):
        if not _is_view_extension(value.type) and _holds_view_extension(value.type):
            raise _refusal("the array", value.type)
        return value
    if not isinstance(value, (pa.RecordBatch, pa.Table, pa.RecordBatchReader)):
        return value
    schema = value.schema
    views = set()
    for at, field in enumerate(schema):
        if _is_view_extension(field.type):
            views.add(at)
        elif _holds_view_extension(field.type):
            raise _refusal(field.name, field.type)
    if not views:
        return value
    storage = pa.schema(
        [_storage_field(field) if at in views else field for at, field in enumerate(schema)],
        metadata=schema.metadata,
    )

    def stored(batch: pa.RecordBatch) -> pa.RecordBatch:
        columns = [
            batch.column(at).storage if at in views else batch.column(at)
            for at in range(batch.num_columns)
        ]
        return pa.RecordBatch.from_arrays(columns, schema=storage)

    if isinstance(value, pa.RecordBatch):
        return stored(value)
    if isinstance(value, pa.Table):
        return pa.Table.from_batches([stored(batch) for batch in value.to_batches()], storage)
    return pa.RecordBatchReader.from_batches(storage, (stored(batch) for batch in value))


def names() -> tuple[str, ...]:
    """Every extension name this module registered with pyarrow."""
    return tuple(_TYPES)


def _register() -> None:
    for name in DataType.ARROW_EXTENSION_NAMES:
        if not name.startswith(PREFIX):
            continue
        registered = type(
            "YggdrylType",
            (YggdrylType,),
            {"_extension_name": name, "__module__": __name__},
        )
        _TYPES[name] = registered
        try:
            pa.register_extension_type(registered(pa.null()))
        except pa.ArrowKeyError:
            # The package was imported again: its own earlier class gives way.
            pa.unregister_extension_type(name)
            pa.register_extension_type(registered(pa.null()))


_register()
