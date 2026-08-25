from __future__ import annotations

import dataclasses
from typing import Annotated

import pyarrow as pa
import pytest

from yggdryl import DataType, Field, field, scalar


def _schema() -> pa.Schema:
    return pa.schema(
        [
            pa.field("identifier", pa.int16(), nullable=False, metadata={"id": "7"}),
            pa.field(
                "payload",
                pa.struct(
                    [
                        pa.field("label", pa.string(), nullable=False),
                        pa.field("score", pa.float32()),
                    ]
                ),
                nullable=False,
            ),
            pa.field("tags", pa.list_(pa.field("item", pa.string(), False))),
        ],
        metadata={"source": "arrow"},
    )


def test_arrow_schema_import_is_one_native_struct_field() -> None:
    schema = _schema()
    root = Field.from_arrow_schema(schema, name="event")

    assert root.name == "event"
    assert not root.nullable
    assert root.data_type.id == "struct"
    assert root.metadata["source"] == "arrow"
    assert tuple(child.name for child in root.data_type) == (
        "identifier",
        "payload",
        "tags",
    )
    assert root.data_type["identifier"].data_type.id == "int16"
    assert root.data_type["payload"].data_type["score"].data_type.id == "float32"
    assert root.into_arrow_schema() == schema
    assert root.into_arrow_schema() == schema


def test_native_field_materializes_plain_nested_dataclasses() -> None:
    root = Field.from_arrow_schema(_schema(), name="event")
    Event = root.into_dataclass(name="Event", module=__name__)

    assert dataclasses.is_dataclass(Event)
    assert Event.__name__ == "Event"
    assert Event.__module__ == __name__
    assert isinstance(Event.__dict__["field"], staticmethod)
    assert Event.field() is root
    assert Event.field() is Event.field()
    assert "FIELD" not in Event.__dict__
    assert not any(base.__name__ == "Record" for base in Event.__mro__)

    members = dataclasses.fields(Event)
    assert tuple(member.name for member in members) == (
        "identifier",
        "payload",
        "tags",
    )
    Payload = members[1].type
    assert dataclasses.is_dataclass(Payload)
    assert Payload.field() == root.data_type["payload"]
    assert Payload.field() is Payload.field()

    value = Event(identifier=1, payload=Payload(label="ok"), tags=None)
    assert value.identifier == 1
    assert value.payload.label == "ok"
    assert value.payload.score is None


def test_generated_class_preserves_narrow_and_nested_native_types() -> None:
    root = Field(
        "row",
        DataType.from_fields(
            (
                Field("small", "int8", nullable=False),
                Field(
                    "nested",
                    DataType.from_fields((Field("amount", "decimal128(12, 2)"),)),
                    nullable=False,
                    metadata={"role": "payload"},
                ),
            )
        ),
        nullable=False,
        metadata={"owner": "root"},
    )
    Row = root.into_dataclass(name="Row")

    assert Row.field() is root
    assert Row.field().data_type["small"].data_type.id == "int8"
    Nested = dataclasses.fields(Row)[1].type
    assert Nested.field() == root.data_type["nested"]
    assert Nested.field() is Nested.field()
    assert Nested.field().metadata["role"] == "payload"
    assert field(Row) is root
    assert root.into_arrow_schema() == Row.field().into_arrow_schema()


def test_decorated_dataclass_round_trips_through_arrow_schema() -> None:
    @scalar
    class Quote:
        symbol: str
        bid: float
        ask: float | None = None

    schema = Quote.field().into_arrow_schema()
    imported = Field.from_arrow_schema(schema, name=Quote.field().name)
    Restored = imported.into_dataclass(name="Restored")

    assert imported == Quote.field()
    assert Restored.field() is imported
    assert tuple(item.name for item in dataclasses.fields(Restored)) == (
        "symbol",
        "bid",
        "ask",
    )
    assert Restored(symbol="ABC", bid=1.0).ask is None


def test_into_dataclass_requires_a_non_nullable_struct_root() -> None:
    for root in (
        Field("scalar", "int64", nullable=False),
        Field("nullable", DataType.from_fields(()), nullable=True),
    ):
        with pytest.raises((TypeError, ValueError), match="non-nullable|Struct"):
            root.into_dataclass()


@pytest.mark.parametrize("invalid", ("class", "with-dash", "__private"))
def test_invalid_python_member_names_are_refused(invalid: str) -> None:
    root = Field.from_arrow_schema(
        pa.schema([pa.field(invalid, pa.int64(), nullable=False)]),
        name="valid_root",
    )
    with pytest.raises(TypeError, match=invalid):
        root.into_dataclass()


def test_invalid_python_root_name_is_refused() -> None:
    root = Field.from_arrow_schema(
        pa.schema([pa.field("value", pa.int64(), nullable=False)]),
        name="invalid-root",
    )
    with pytest.raises(TypeError, match="invalid-root"):
        root.into_dataclass()


def test_field_is_available_as_a_generated_instance_member() -> None:
    root = Field.from_arrow_schema(
        pa.schema([pa.field("FIELD", pa.int64(), nullable=False)]),
        name="row",
    )
    Row = root.into_dataclass(name="RowWithFieldMember")

    assert not isinstance(Row.__dict__["FIELD"], Field)
    assert Row(FIELD=7).FIELD == 7
    assert Row.field() is root


def test_field_is_reserved_for_generated_classes() -> None:
    root = Field.from_arrow_schema(
        pa.schema([pa.field("field", pa.int64(), nullable=False)]),
        name="row",
    )

    with pytest.raises(TypeError, match="field"):
        root.into_dataclass()


def test_arrow_schema_round_trip_preserves_sorted_map_layout() -> None:
    schema = pa.schema(
        [
            pa.field(
                "lookup",
                pa.map_(pa.string(), pa.int32(), keys_sorted=True),
                nullable=False,
            )
        ]
    )
    root = Field.from_arrow_schema(schema, name="row")

    assert root.data_type["lookup"].data_type.into_arrow().keys_sorted
    assert root.into_arrow_schema() == schema


def test_arrow_schema_round_trip_preserves_dictionary_identity() -> None:
    dictionary = Field.from_pyhint(
        "symbol",
        Annotated[
            str,
            ("arrow_type", pa.dictionary(pa.int16(), pa.string(), ordered=True)),
            ("dictionary_id", 29),
            ("dictionary_is_ordered", True),
        ],
    )
    root = Field(
        "row",
        DataType.from_fields((dictionary,)),
        nullable=False,
    )
    restored = Field.from_arrow_schema(root.into_arrow_schema(), name="row")

    assert restored.data_type["symbol"].dictionary_id == 29
    assert restored.data_type["symbol"].dictionary_is_ordered is True


def test_arrow_schema_round_trip_rehydrates_registered_extension_identity() -> None:
    class SchemaExtension(pa.ExtensionType):
        def __init__(self) -> None:
            super().__init__(pa.int32(), "tests.field-class.schema-extension")

        def __arrow_ext_serialize__(self) -> bytes:
            return b"v1"

        @classmethod
        def __arrow_ext_deserialize__(
            cls, storage_type: pa.DataType, serialized: bytes
        ) -> SchemaExtension:
            assert storage_type == pa.int32()
            assert serialized == b"v1"
            return cls()

    extension = SchemaExtension()
    pa.register_extension_type(extension)
    try:
        schema = pa.schema(
            [pa.field("payload", extension, nullable=False)],
            metadata={"source": "extension"},
        )
        root = Field.from_arrow_schema(schema, name="row")
        exported = root.into_arrow_schema()
        restored = Field.from_arrow_schema(exported, name="row")

        assert exported.field("payload").type == extension
        assert restored.data_type["payload"].into_arrow().type == extension
        assert restored.metadata["source"] == "extension"
    finally:
        pa.unregister_extension_type(extension.extension_name)


def test_field_renames_a_pyarrow_dictionary_field_without_losing_options() -> None:
    arrow = pa.field(
        "symbol",
        pa.dictionary(pa.int16(), pa.large_string(), ordered=True),
        nullable=False,
        metadata={"source": "arrow"},
    )
    native = field(arrow, name="venue")

    assert native.name == "venue"
    assert not native.nullable
    assert native.metadata["source"] == "arrow"
    assert native.dictionary_is_ordered is True
    assert native.into_arrow().type == arrow.type


def test_field_bare_pyarrow_dictionary_type_preserves_ordering() -> None:
    arrow = pa.dictionary(pa.uint8(), pa.string(), ordered=True)
    native = field(arrow, name="category")

    assert native.name == "category"
    assert native.nullable
    assert native.dictionary_is_ordered is True
    assert native.into_arrow().type == arrow


def test_field_name_is_uniform_for_every_arrow_shape() -> None:
    schema = pa.schema([pa.field("value", pa.int64(), nullable=False)])

    assert field(schema, name="").name == ""
    assert field(pa.int64(), name="").name == ""
    for value in (schema, schema.field(0), pa.int64()):
        with pytest.raises(TypeError, match="name must be str or None"):
            field(value, name=7)  # type: ignore[arg-type]
