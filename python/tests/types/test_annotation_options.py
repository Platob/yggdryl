from __future__ import annotations

import dataclasses
import typing
from decimal import Decimal
from typing import Annotated, ClassVar

import pyarrow as pa
import pytest
import typing_extensions

from yggdryl import DataType, Field, field, scalar


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
        "python.class": "Decimal",
        "python.kind": "class",
        "python.module": "decimal",
        "python.qualname": "Decimal",
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
    from yggdryl.types import _hints

    def unavailable() -> typing.NoReturn:
        raise AssertionError("ordinary inference imported PyArrow")

    _hints._pyarrow_module.cache_clear()
    monkeypatch.setattr(_hints, "_pyarrow_module", unavailable)
    assert DataType.from_pyhint(list[dict[str, int]]).id == "list"
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

    root = Envelope.field()
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
    assert not Child.field().dtype["required"].nullable
    assert Child.field().dtype["relaxed"].nullable


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

    physical = Mismatch.field().dtype["child"].dtype
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

    physical = Misaligned.field().dtype["value"].dtype
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

    root = Aliases.field()
    assert root.dtype["maybe"].nullable
    assert root.dtype["either"].dtype.id == "union"
    assert root.dtype["generic"].nullable


def test_a_field_annotation_contributes_its_metadata() -> None:
    tag = Field("value", "int64", metadata={"unit": "ms", "iceberg:doc": "elapsed"})

    @scalar
    class Reading:
        value: Annotated[int, tag]

    column = Reading.field().dtype["value"]
    assert column.metadata["unit"] == "ms"
    assert column.metadata["iceberg:doc"] == "elapsed"
