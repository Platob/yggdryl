"""Annotations: the datatype a hint names, and the options it carries."""

from __future__ import annotations

import collections
import collections.abc as cabc
import dataclasses
import datetime
import decimal
import enum
import pathlib
import typing
import uuid
from dataclasses import dataclass
from decimal import Decimal
from typing import (
    Annotated,
    Any,
    ClassVar,
    Generic,
    Literal,
    NamedTuple,
    NewType,
    TypeVar,
    TypedDict,
)

import pyarrow as pa
import pytest
import typing_extensions

from yggdryl import DataType, Field, Uri, Url, Urn, field, scalar

def test_field_options_resolve_left_to_right_before_caller_metadata() -> None:
    hint = Annotated[
        Decimal | None,
        ("arrow_type", pa.decimal128(9, 0)),
        {"nullable": True, "metadata": {"source": "first", "unit": "usd"}},
        ("nullable", False),
        ("metadata", {"source": "last", "nullable": "metadata-value"}),
        ("id", 7),
    ]

    field = Field.from_pyhint(
        "price",
        hint,
        metadata={"source": "caller", "role": "settlement"},
    )

    assert field.into_arrow().type == pa.decimal128(9, 0)
    assert not field.nullable
    assert field.parquet_field_id == 7
    assert dict(field.metadata.items()) == {
        "PARQUET:field_id": "7",
        "nullable": "metadata-value",
        "PYTHON:kind": "class",
        "PYTHON:module": "decimal",
        "PYTHON:qualname": "Decimal",
        "role": "settlement",
        "source": "caller",
        "unit": "usd",
    }

    canonical_id = Field.from_pyhint(
        "id",
        Annotated[
            int,
            ("id", 3),
            ("metadata", {"PARQUET:field_id": "+003"}),
        ],
    )
    assert canonical_id.parquet_field_id == 3
    with pytest.raises(TypeError, match=r"conflicting Annotated id"):
        Field.from_pyhint(
            "id",
            Annotated[
                int,
                ("id", 4),
                ("metadata", {"PARQUET:field_id": "3"}),
            ],
        )


def test_dtype_accepts_only_arrow_type_and_only_real_pyarrow_types() -> None:
    precise = DataType.from_pyhint(
        Annotated[Decimal, {"arrow_type": pa.decimal256(45, 8)}]
    )
    assert precise.into_arrow() == pa.decimal256(45, 8)

    for hint in (
        Annotated[int, ("arrow_type", "int8")],
        Annotated[int, ("arrow_type", DataType.from_str("int8"))],
        Annotated[int, ("arrow_type", Field("x", "int8"))],
    ):
        with pytest.raises(TypeError, match=r"arrow_type.*pyarrow\.DataType"):
            DataType.from_pyhint(hint)

    for hint in (
        Annotated[float, ("nullable", 1)],
        Annotated[int, ("dictionary_id", 1)],
        Annotated[int, {"metadata": {"role": "value"}}],
    ):
        with pytest.raises(TypeError, match=r"apply to a Field.*Field\.from_pyhint"):
            DataType.from_pyhint(hint)

    # Unrecognized string annotation metadata is inert for a bare datatype.
    assert DataType.from_pyhint(Annotated[int, {"unit": "items"}]).id == "int64"


def test_ordinary_inference_does_not_touch_pyarrow_override_boundary(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    from yggdryl import _hints

    def unavailable() -> typing.NoReturn:
        raise AssertionError("ordinary inference imported PyArrow")

    _hints._pyarrow_module.cache_clear()
    monkeypatch.setattr(_hints, "_pyarrow_module", unavailable)
    assert DataType.from_pyhint(list[dict[str, int]]).id == "serie"
    assert Field.from_pyhint("value", int).dtype.id == "int64"


def test_dictionary_options_are_complete_exact_and_native_validated() -> None:
    hint = Annotated[
        str,
        ("arrow_type", pa.dictionary(pa.int16(), pa.string(), ordered=False)),
        ("dictionary_id", 29),
        ("dictionary_is_ordered", True),
    ]
    field = Field.from_pyhint("symbol", hint)
    assert field.dictionary_id == 29
    assert field.dictionary_is_ordered is True
    assert field.into_arrow().type == pa.dictionary(pa.int16(), pa.string(), ordered=True)

    with pytest.raises(TypeError, match="must appear together"):
        Field.from_pyhint("bad", Annotated[int, {"dictionary_id": 1}])
    with pytest.raises(TypeError, match="must both be supplied"):
        Field.from_pyhint("bad", Annotated[int, ("dictionary_id", 1)])
    with pytest.raises(TypeError, match=r"dictionary options at bad"):
        Field.from_pyhint(
            "bad",
            Annotated[
                int,
                ("dictionary_id", 1),
                ("dictionary_is_ordered", False),
            ],
        )
    with pytest.raises(TypeError, match="nullable option.*bool"):
        Field.from_pyhint("bad", Annotated[bytes, ("nullable", 1)])
    with pytest.raises(TypeError, match="dictionary_id option.*int"):
        Field.from_pyhint(
            "bad",
            Annotated[
                str,
                ("arrow_type", pa.dictionary(pa.int8(), pa.string())),
                ("dictionary_id", True),
                ("dictionary_is_ordered", False),
            ],
        )


def test_only_final_structural_option_values_are_validated() -> None:
    field = Field.from_pyhint(
        "value",
        Annotated[
            int,
            ("nullable", "stale"),
            ("nullable", True),
            ("id", 2**40),
            ("id", 7),
        ],
    )
    assert field.nullable
    assert field.parquet_field_id == 7

    dictionary = Field.from_pyhint(
        "value",
        Annotated[
            str,
            ("arrow_type", "stale"),
            ("arrow_type", pa.dictionary(pa.int8(), pa.string())),
            ("dictionary_id", "stale"),
            ("dictionary_is_ordered", "stale"),
            ("dictionary_id", 3),
            ("dictionary_is_ordered", False),
        ],
    )
    assert dictionary.dictionary_id == 3
    assert dictionary.dictionary_is_ordered is False

    with pytest.raises(TypeError, match="nullable option.*bool"):
        Field.from_pyhint(
            "value",
            Annotated[int, ("nullable", True), ("nullable", "final")],
        )
    with pytest.raises(ValueError, match="id option.*2147483647"):
        Field.from_pyhint(
            "value",
            Annotated[int, ("id", 7), ("id", 2**40)],
        )

    for malformed in (("arrow_type",), ("nullable", True, "extra")):
        with pytest.raises(TypeError, match=r"must be exactly \(key, value\)"):
            Field.from_pyhint("value", Annotated[int, malformed])

    aliased_inner = typing_extensions.TypeAliasType(
        "AliasedInner",
        Annotated[
            int,
            ("id", 1),
            ("metadata", {"layer": "inner", "inner": "kept"}),
        ],
    )
    alias_overlay = Field.from_pyhint(
        "value",
        Annotated[
            aliased_inner,
            ("id", 2),
            ("metadata", {"layer": "outer"}),
        ],
    )
    assert alias_overlay.parquet_field_id == 2
    assert alias_overlay.metadata["layer"] == "outer"
    assert alias_overlay.metadata["inner"] == "kept"


class _AnnotationExtension(pa.ExtensionType):
    def __init__(self) -> None:
        super().__init__(pa.int32(), "yggdryl.tests.annotation-extension")

    def __arrow_ext_serialize__(self) -> bytes:
        return b"v1"

    @classmethod
    def __arrow_ext_deserialize__(
        cls, storage_type: pa.DataType, serialized: bytes
    ) -> _AnnotationExtension:
        del storage_type, serialized
        return cls()


class _BinaryAnnotationExtension(pa.ExtensionType):
    def __init__(self) -> None:
        super().__init__(pa.int32(), "yggdryl.tests.binary-annotation-extension")

    def __arrow_ext_serialize__(self) -> bytes:
        return b"\xff\x00binary"

    @classmethod
    def __arrow_ext_deserialize__(
        cls, storage_type: pa.DataType, serialized: bytes
    ) -> _BinaryAnnotationExtension:
        del storage_type, serialized
        return cls()


def test_extension_override_preserves_identity_and_protects_metadata() -> None:
    extension = _AnnotationExtension()
    try:
        pa.unregister_extension_type(extension.extension_name)
    except pa.ArrowKeyError:
        pass
    pa.register_extension_type(extension)
    try:
        field = Field.from_pyhint(
            "code",
            Annotated[
                int,
                ("arrow_type", extension),
                ("metadata", {"owner": "test"}),
            ],
        )
        assert field.into_arrow().type == extension
        assert field.metadata["owner"] == "test"

        member = Annotated[
            int,
            ("arrow_type", extension),
            ("id", 17),
            ("metadata", {"member": "preserved"}),
        ]
        promoted = Field.from_pyhint("code", member | None)
        assert promoted.into_arrow().type == extension
        assert promoted.parquet_field_id == 17
        assert promoted.metadata["member"] == "preserved"

        identical = Field.from_pyhint(
            "code",
            Annotated[
                int,
                ("arrow_type", extension),
                ("metadata", {"ARROW:extension:name": extension.extension_name}),
            ],
        )
        assert identical.into_arrow().type == extension

        with pytest.raises(TypeError, match="conflicts.*ExtensionType"):
            Field.from_pyhint(
                "code",
                Annotated[
                    int,
                    ("arrow_type", extension),
                    ("metadata", {"ARROW:extension:name": "corrupt"}),
                ],
            )
        with pytest.raises(TypeError, match=r"use Field\.from_pyhint"):
            DataType.from_pyhint(
                Annotated[int, ("arrow_type", extension)]
            )
    finally:
        pa.unregister_extension_type(extension.extension_name)

    binary = _BinaryAnnotationExtension()
    try:
        pa.unregister_extension_type(binary.extension_name)
    except pa.ArrowKeyError:
        pass
    pa.register_extension_type(binary)
    try:
        with pytest.raises(TypeError, match=r"code.*UTF-8|UTF-8.*code"):
            Field.from_pyhint(
                "code", Annotated[int, ("arrow_type", binary)]
            )
    finally:
        pa.unregister_extension_type(binary.extension_name)


def test_nullable_options_compile_recursively_into_native_fields() -> None:
    @scalar
    class Child:
        required: Annotated[int | None, ("nullable", False)]
        relaxed: Annotated[int, ("nullable", True)]

    @scalar
    class Envelope:
        children: list[Annotated[Child, ("nullable", True)]]
        values: dict[str, Annotated[int, ("nullable", True)]]
        choice: Annotated[Child | str, ("nullable", True)]

    root = Envelope.into_field()
    children = root.dtype["children"].dtype[0]
    values = (
        root.dtype["values"]
        .dtype[0]
        .dtype["value"]
    )
    choice = root.dtype["choice"]

    assert children.nullable
    assert values.nullable
    assert choice.nullable
    assert not Child.into_field().dtype["required"].nullable
    assert Child.into_field().dtype["relaxed"].nullable


def test_parent_arrow_type_owns_the_subtree() -> None:
    shadowed = Annotated[
        list[Annotated[int, ("arrow_type", object()), ("nullable", False)]],
        ("arrow_type", pa.list_(pa.field("item", pa.int8(), nullable=True))),
    ]
    native = Field.from_pyhint("values", shadowed)
    assert native.into_arrow().type == pa.list_(
        pa.field("item", pa.int8(), nullable=True)
    )

    @scalar
    class Logical:
        value: int

    @scalar
    class Mismatch:
        child: Annotated[
            Logical,
            (
                "arrow_type",
                pa.struct([pa.field("different", pa.int64(), nullable=False)]),
            ),
        ]

    physical = Mismatch.into_field().dtype["child"].dtype
    assert tuple(child.name for child in physical) == ("different",)
    assert physical["different"].dtype.id == "int64"


def test_non_materialized_dataclass_options_are_rejected() -> None:
    @dataclasses.dataclass
    class WithInitVar:
        transient: dataclasses.InitVar[Annotated[int, ("nullable", True)]]

    @dataclasses.dataclass
    class WithClassVar:
        shared: ClassVar[Annotated[int, ("arrow_type", pa.int8())]] = 1

    for candidate in (WithInitVar, WithClassVar):
        with pytest.raises(TypeError, match=r"InitVar and ClassVar.*not schema fields"):
            field(candidate)


def test_explicit_union_override_is_the_physical_authority() -> None:
    one_child = pa.union(
        [pa.field("integer", pa.int64(), nullable=False)],
        mode="dense",
        type_codes=[0],
    )

    @scalar
    class Misaligned:
        value: Annotated[int | str, ("arrow_type", one_child)]

    physical = Misaligned.into_field().dtype["value"].dtype
    assert physical.id == "union"
    assert len(physical) == 1
    assert physical[0].name == "integer"


def test_pep695_aliases_compile_to_optional_and_union_fields() -> None:
    Maybe = typing_extensions.TypeAliasType("Maybe", int | None)
    Either = typing_extensions.TypeAliasType("Either", int | str)
    ValueT = typing.TypeVar("ValueT")
    GenericMaybe = typing_extensions.TypeAliasType(
        "GenericMaybe", ValueT | None, type_params=(ValueT,)
    )

    @scalar
    class Aliases:
        maybe: Maybe
        either: Either
        generic: GenericMaybe[int]

    root = Aliases.into_field()
    assert root.dtype["maybe"].nullable
    assert root.dtype["either"].dtype.id == "union"
    assert root.dtype["generic"].nullable


def test_a_field_annotation_contributes_its_metadata() -> None:
    tag = Field("value", "int64", metadata={"unit": "ms", "ICEBERG:doc": "elapsed"})

    @scalar
    class Reading:
        value: Annotated[int, tag]

    column = Reading.into_field().dtype["value"]
    assert column.metadata["unit"] == "ms"
    assert column.metadata["ICEBERG:doc"] == "elapsed"


class QuoteDict(TypedDict):
    symbol: str
    price: decimal.Decimal
    venue: str | None


class Point(NamedTuple):
    x: float
    y: float


@dataclass
class Quote:
    symbol: str
    sizes: list[int | None]
    point: Point


T = TypeVar("T")


@dataclass
class Box(Generic[T]):
    value: T


@dataclass
class IntBox(Box[int]):
    label: str = ""


class Side(enum.Enum):
    BID = "bid"
    ASK = "ask"


class Mixed(enum.Enum):
    ENABLED = True
    COUNT = 2


UserId = NewType("UserId", int)


def test_scalar_hints_have_native_arrow_equivalents() -> None:
    expected = {
        Any: "null",
        object: "null",
        type(None): "null",
        bool: "boolean",
        int: "int64",
        float: "float64",
        str: "utf8",
        bytes: "binary",
        bytearray: "binary",
        memoryview: "binary",
        datetime.datetime: "datetime64",
        datetime.date: "date32",
        datetime.time: "time64",
        datetime.timedelta: "duration64",
        decimal.Decimal: "decimal",
        uuid.UUID: "uuid",
        pathlib.Path: "utf8",
        Uri: "utf8",
        Url: "utf8",
        Urn: "utf8",
    }

    for hint, kind in expected.items():
        assert DataType.from_pyhint(hint).id == kind

    assert DataType.from_pyhint(datetime.datetime).into_arrow() == pa.timestamp(
        "us", tz="UTC"
    )
    assert DataType.from_pyhint(decimal.Decimal).into_arrow() == pa.decimal128(38, 18)


def test_scalar_subclasses_keep_their_physical_type() -> None:
    class Count(int):
        note: str

    class EventTime(datetime.datetime):
        pass

    class Price(decimal.Decimal):
        pass

    assert DataType.from_pyhint(Count).id == "int64"
    assert DataType.from_pyhint(EventTime).id == "datetime64"
    assert DataType.from_pyhint(Price).id == "decimal"


def test_only_explicit_none_makes_fields_nullable() -> None:
    assert not Field.from_pyhint("plain", int).nullable
    assert not Field.from_pyhint("default_is_not_inspected", Any).nullable
    assert Field.from_pyhint("optional", int | None).nullable
    assert Field.from_pyhint("none", None).nullable
    assert Field.from_pyhint("literal", Literal["ok", None]).nullable
    assert not Field.from_pyhint("literal", Literal["ok"]).nullable
    assert DataType.from_pyhint(int | None) == DataType.from_pyhint(int)


def test_annotated_metadata_is_string_only_and_explicit_values_win() -> None:
    hint = Annotated[int | None, {"unit": "lots", "source": "annotation"}]
    field = Field.from_pyhint(
        "quantity",
        hint,
        metadata={"source": "caller", "role": "size"},
    )

    assert field.nullable
    assert dict(field.metadata.items()) == {
        "role": "size",
        "source": "caller",
        "unit": "lots",
    }
    with pytest.raises(TypeError, match="str keys to str values"):
        Field.from_pyhint("bad", Annotated[int, {"precision": 4}])


def test_collection_hints_preserve_nested_nullability_and_order() -> None:
    listed = DataType.from_pyhint(list[int | None])
    fixed = DataType.from_pyhint(tuple[int, str | None])
    mapping = DataType.from_pyhint(dict[str, int | None])

    assert listed.id == "serie"
    assert listed[0].name == "item"
    assert listed[0].nullable
    assert [field.name for field in fixed] == ["_1", "_2"]
    assert not fixed[0].nullable
    assert fixed[1].nullable
    assert mapping.id == "map"
    entries = mapping[0].dtype
    assert [field.name for field in entries] == ["key", "value"]
    assert not entries[0].nullable
    assert entries[1].nullable
    assert DataType.from_arrow(mapping.into_arrow()) == mapping

    assert DataType.from_pyhint(cabc.Iterable).id == "serie"
    assert DataType.from_pyhint(typing.Tuple).id == "serie"
    assert DataType.from_pyhint(tuple[()]).id == "struct"
    assert DataType.from_pyhint(cabc.Generator[str, None, None])[0].dtype.id == "utf8"
    items = DataType.from_pyhint(cabc.ItemsView[str, int])
    assert [field.name for field in items[0].dtype] == ["_1", "_2"]
    with pytest.raises(TypeError, match="nullable map key"):
        DataType.from_pyhint(dict[str | None, int])


def test_nested_hint_inference_uses_native_builders_without_pyarrow_round_trips(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    import yggdryl._hints as hint_impl

    def unexpected_arrow_factory(*args: object, **kwargs: object) -> object:
        raise AssertionError(
            f"annotation inference called a PyArrow schema factory: {args!r}, {kwargs!r}"
        )

    for name in ("list_", "map_", "struct", "union"):
        monkeypatch.setattr(pa, name, unexpected_arrow_factory)
    original_import_module = hint_impl.importlib.import_module

    def import_without_pyarrow(name: str, package: str | None = None) -> object:
        if name == "pyarrow":
            raise AssertionError("annotation inference imported PyArrow")
        return original_import_module(name, package)

    monkeypatch.setattr(hint_impl.importlib, "import_module", import_without_pyarrow)

    @dataclass
    class NativeNested:
        labels: list[Annotated[int | None, {"unit": "ticks"}]]
        dimensions: dict[str, int]

    inferred = DataType.from_pyhint(NativeNested)
    labels = inferred["labels"].dtype
    dimensions = inferred["dimensions"].dtype

    assert inferred.id == "struct"
    assert labels.id == "serie"
    assert labels[0].nullable
    assert labels[0].metadata["unit"] == "ticks"
    assert dimensions.id == "map"
    assert [field.name for field in dimensions[0].dtype] == ["key", "value"]

    deep: object = Annotated[int, {"depth": "leaf"}]
    for _ in range(16):
        deep = list[deep]  # type: ignore[valid-type]
    current = DataType.from_pyhint(deep)
    leaf: Field | None = None
    for _ in range(16):
        assert current.id == "serie"
        leaf = current[0]
        current = leaf.dtype
    assert leaf is not None
    assert current.id == "int64"
    assert leaf.metadata["depth"] == "leaf"


def test_items_view_and_union_inference_preserve_native_child_state() -> None:
    items = DataType.from_pyhint(
        cabc.ItemsView[Annotated[str, {"role": "key"}], int | None]
    )
    pair = items[0].dtype
    union = DataType.from_pyhint(Annotated[int, {"source": "integer"}] | str)

    assert items.id == "serie"
    assert pair.id == "struct"
    assert pair[0].metadata["role"] == "key"
    assert pair[1].nullable
    assert union.id == "union"
    assert union[0].metadata["source"] == "integer"
    assert [field.dictionary_id for field in union] == [None, None]
    assert union.into_arrow().mode == "dense"
    assert tuple(union.into_arrow().type_codes) == (0, 1)


def test_none_among_several_members_is_a_null_member_appended_last() -> None:
    # One value type beside `None` stays that type; the field is what may be
    # absent.
    assert DataType.from_pyhint(int | None) == DataType.from_pyhint(int)
    # Several: an Arrow union has no validity, so `None` is a member of its
    # own, appended so the value members keep their type ids.
    for hint in (int | str | None, typing.Optional[int | str]):
        union = DataType.from_pyhint(hint)
        assert union.id == "union"
        assert [member.name for member in union] == ["int", "str", "NoneType"]
        assert [member.dtype.id for member in union] == ["int64", "utf8", "null"]
        assert [member.nullable for member in union] == [False, False, True]
        assert tuple(union.into_arrow().type_codes) == (0, 1, 2)
    # Both spellings of the one hint infer one field: a union is no class to
    # declare.
    assert Field.from_pyhint("value", int | str | None) == Field.from_pyhint(
        "value", typing.Optional[int | str]
    )
    item = DataType.from_pyhint(list[int | str | None])[0].dtype
    assert [member.dtype.id for member in item] == ["int64", "utf8", "null"]


def test_deep_union_inference_assigns_exact_tags_at_each_variant_boundary() -> None:
    hint = list[
        dict[str, Annotated[int, {"branch": "count"}] | str]
        | tuple[bytes, float]
    ]

    inferred = DataType.from_pyhint(hint)
    outer = inferred[0].dtype
    mapping = outer[0].dtype
    mapping_value = mapping[0].dtype[1].dtype

    assert inferred.id == "serie"
    assert outer.id == "union"
    assert outer.into_arrow().mode == "dense"
    assert tuple(outer.into_arrow().type_codes) == (0, 1)
    assert [member.dtype.id for member in outer] == ["map", "struct"]
    assert mapping_value.id == "union"
    assert tuple(mapping_value.into_arrow().type_codes) == (0, 1)
    assert mapping_value[0].metadata["branch"] == "count"


def test_counter_and_generic_mapping_subclasses_keep_parameters() -> None:
    counter = DataType.from_pyhint(collections.Counter[str])
    chain = DataType.from_pyhint(collections.ChainMap[str, int])

    counter_entries = counter[0].dtype
    chain_entries = chain[0].dtype
    assert [field.dtype.id for field in counter_entries] == ["utf8", "int64"]
    assert [field.dtype.id for field in chain_entries] == ["utf8", "int64"]


def test_struct_hints_are_deterministic_and_keep_class_identity() -> None:
    quote_dict = DataType.from_pyhint(QuoteDict)
    point = DataType.from_pyhint(Point)
    quote = DataType.from_pyhint(Quote)
    boxed = DataType.from_pyhint(Box[int])
    inherited_box = DataType.from_pyhint(IntBox)

    assert [field.name for field in quote_dict] == ["symbol", "price", "venue"]
    assert quote_dict[2].nullable
    assert [field.name for field in point] == ["x", "y"]
    assert [field.name for field in quote] == ["symbol", "sizes", "point"]
    assert boxed[0].dtype.id == "int64"
    assert inherited_box[0].dtype.id == "int64"

    root = Field.from_pyhint("quote", Quote)
    declared = root.python.class_metadata
    assert declared is not None
    assert declared.module == __name__
    # The bare name is derived from the qualified one, never stored beside it.
    assert declared.class_name == "Quote"
    assert declared.qualname == "Quote"
    assert declared.kind == "dataclass"
    assert declared.import_path == f"{__name__}.Quote"
    assert declared.is_importable


def test_literal_enum_newtype_typevar_and_union_inference() -> None:
    constrained = TypeVar("constrained", int, str)
    bounded = TypeVar("bounded", bound=float)

    assert DataType.from_pyhint(Literal[1, 2]).id == "int64"
    assert DataType.from_pyhint(Side).id == "utf8"
    assert DataType.from_pyhint(UserId).id == "int64"
    assert DataType.from_pyhint(bounded).id == "float64"

    union = DataType.from_pyhint(constrained)
    assert union.id == "union"
    assert [field.name for field in union] == ["int", "str"]

    # bool is deliberately not collapsed into Python's int physical type.
    mixed = DataType.from_pyhint(Mixed)
    assert mixed.id == "union"
    assert [field.dtype.id for field in mixed] == ["boolean", "int64"]


def test_typing_extensions_wrappers_match_stdlib_semantics() -> None:
    typing_extensions = pytest.importorskip("typing_extensions")
    required = typing_extensions.Required[int]
    optional_key = typing_extensions.NotRequired[str | None]
    annotated = typing_extensions.Annotated[int, {"unit": "ticks"}]

    assert not Field.from_pyhint("required", required).nullable
    assert Field.from_pyhint("optional_key", optional_key).nullable
    assert Field.from_pyhint("annotated", annotated).metadata["unit"] == "ticks"


def test_internal_namespace_resolves_deep_local_struct_annotations() -> None:
    typing_extensions = pytest.importorskip("typing_extensions")
    ReadOnly = typing_extensions.ReadOnly
    LocalScalar = Annotated[int, {"unit": "local"}]

    class LocalPayload(TypedDict):
        value: ReadOnly[LocalScalar]

    class LocalPair(NamedTuple):
        payload: LocalPayload

    @dataclass
    class LocalEnvelope:
        pair: LocalPair

    Shadowed = str

    @dataclass
    class LocalShadow:
        Shadowed = int
        value: Shadowed

    from yggdryl._hints import _field_from_pyhint

    root = _field_from_pyhint(
        "envelope",
        LocalEnvelope,
        localns=locals(),
    )
    value = root.dtype["pair"].dtype["payload"].dtype["value"]
    assert value.dtype.id == "int64"
    assert value.metadata["unit"] == "local"
    shadow = _field_from_pyhint(
        "shadow",
        LocalShadow,
        localns=locals(),
    )
    assert shadow.dtype["value"].dtype.id == "int64"


@pytest.mark.skipif(
    not hasattr(typing, "TypeAliasType"), reason="PEP 695 requires Python 3.12+"
)
def test_pep695_alias_members_preserve_none_and_annotated_metadata() -> None:
    namespace: dict[str, object] = {"Annotated": Annotated, "__name__": __name__}
    exec(
        "type Nil = None\n"
        "type Maybe[T] = T | None\n"
        "type Tagged[T] = Annotated[T, {'unit': 'alias'}]\n"
        "type Fixed = Annotated[int, {'source': 'fixed'}]\n",
        namespace,
    )
    nil = namespace["Nil"]
    maybe = namespace["Maybe"]
    tagged = namespace["Tagged"]
    fixed = namespace["Fixed"]

    alias_member = Field.from_pyhint("value", int | nil)  # type: ignore[operator]
    generic_optional = Field.from_pyhint("value", maybe[int])  # type: ignore[index]
    generic_tagged = Field.from_pyhint("value", tagged[int])  # type: ignore[index]
    fixed_tagged = Field.from_pyhint("value", fixed)

    assert alias_member.nullable and alias_member.dtype.id == "int64"
    assert generic_optional.nullable and generic_optional.dtype.id == "int64"
    assert generic_tagged.metadata["unit"] == "alias"
    assert fixed_tagged.metadata["source"] == "fixed"
    assert generic_tagged.python.kind == "type_alias"


def test_recursive_deep_and_unresolved_annotations_fail_cleanly() -> None:
    @dataclass
    class Node:
        child: Node | None

    with pytest.raises(TypeError, match="recursive"):
        DataType.from_pyhint(Node)

    nested: object = int
    for _ in range(70):
        nested = list[nested]  # type: ignore[valid-type]
    with pytest.raises(TypeError, match="depth"):
        DataType.from_pyhint(nested)

    with pytest.raises(TypeError, match="unresolved forward"):
        DataType.from_pyhint("MissingType")
    assert DataType.from_pyhint("int").id == "int64"
