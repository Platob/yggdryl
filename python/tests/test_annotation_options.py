from __future__ import annotations

import dataclasses
import datetime as dt
import typing
from decimal import Decimal
from typing import Annotated, ClassVar

import pyarrow as pa
import pytest
import typing_extensions

from yggdryl import DataType, Field
from yggdryl.records import from_dict, record, to_dict


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

    assert field.to_arrow().type == pa.decimal128(9, 0)
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


def test_data_type_accepts_only_arrow_type_and_only_real_pyarrow_types() -> None:
    precise = DataType.from_pyhint(
        Annotated[Decimal, {"arrow_type": pa.decimal256(45, 8)}]
    )
    assert precise.to_arrow() == pa.decimal256(45, 8)

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

    # Legacy all-string annotation metadata stays harmless for a bare datatype.
    assert DataType.from_pyhint(Annotated[int, {"unit": "items"}]).id == "int64"


def test_ordinary_inference_does_not_touch_pyarrow_override_boundary(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    from yggdryl.records import _hints

    def unavailable() -> typing.NoReturn:
        raise AssertionError("ordinary inference imported PyArrow")

    _hints._pyarrow_module.cache_clear()
    monkeypatch.setattr(_hints, "_pyarrow_module", unavailable)
    assert DataType.from_pyhint(list[dict[str, int]]).id == "list"
    assert Field.from_pyhint("value", int).data_type.id == "int64"


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
    assert field.to_arrow().type == pa.dictionary(pa.int16(), pa.string(), ordered=True)

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
        assert field.to_arrow().type == extension
        assert field.metadata["owner"] == "test"

        member = Annotated[
            int,
            ("arrow_type", extension),
            ("id", 17),
            ("metadata", {"member": "preserved"}),
        ]
        promoted = Field.from_pyhint("code", member | None)
        assert promoted.to_arrow().type == extension
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
        assert identical.to_arrow().type == extension

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


def test_nullable_options_govern_safe_conversion_recursively() -> None:
    @record
    class Child:
        required: Annotated[int | None, ("nullable", False)]
        relaxed: Annotated[int, ("nullable", True)]

    @record
    class Envelope:
        children: list[Annotated[Child, ("nullable", True)]]
        values: dict[str, Annotated[int, ("nullable", True)]]
        choice: Annotated[Child | str, ("nullable", True)]

    values = {
        "children": [None, {"required": "3", "relaxed": None}],
        "values": {"missing": None},
        "choice": None,
    }
    converted = from_dict(Envelope, values)
    assert to_dict(converted) == {
        "children": [None, {"required": 3, "relaxed": None}],
        "values": {"missing": None},
        "choice": None,
    }

    with pytest.raises(TypeError, match=r"Child\.required.*not nullable"):
        to_dict(Envelope([Child(None, 1)], {}, "ok"))
    with pytest.raises(TypeError, match=r"required.*not nullable"):
        from_dict(
            Envelope,
            {
                "children": [{"required": None, "relaxed": 1}],
                "values": {},
                "choice": "ok",
            },
        )


def test_parent_arrow_type_owns_subtree_and_mismatched_names_reject() -> None:
    shadowed = Annotated[
        list[Annotated[int, ("arrow_type", object()), ("nullable", False)]],
        (
            "arrow_type",
            pa.list_(pa.field("item", pa.int8(), nullable=True)),
        ),
    ]
    field = Field.from_pyhint("values", shadowed)
    assert field.to_arrow().type == pa.list_(
        pa.field("item", pa.int8(), nullable=True)
    )

    @record
    class Logical:
        value: int

    @record
    class Mismatch:
        child: Annotated[
            Logical,
            (
                "arrow_type",
                pa.struct([pa.field("different", pa.int64(), nullable=False)]),
            ),
        ]

    with pytest.raises(TypeError, match="do not match physical struct children"):
        Mismatch.from_dict({"child": {"value": 1}})

    @record
    class ExtraPhysical:
        child: Annotated[
            Logical,
            (
                "arrow_type",
                pa.struct(
                    [
                        pa.field("value", pa.int64(), nullable=False),
                        pa.field("extra", pa.string(), nullable=True),
                    ]
                ),
            ),
        ]

    with pytest.raises(TypeError, match=r"do not match physical struct children"):
        ExtraPhysical.from_dict({"child": {"value": 1}})
    with pytest.raises(TypeError, match=r"do not match physical struct children"):
        ExtraPhysical(Logical(1)).to_dict()
    with pytest.raises(TypeError, match=r"do not match physical struct children"):
        ExtraPhysical.into_arrow_record_batch([ExtraPhysical(Logical(1))])

    class Recursive:
        child: Recursive

    with pytest.raises(TypeError, match="recursive Python annotation"):
        Field.from_pyhint(
            "recursive",
            Annotated[Recursive, ("arrow_type", pa.int64())],
        )


def test_non_materialized_dataclass_options_reject() -> None:
    with pytest.raises(TypeError, match=r"InitVar and ClassVar.*not schema fields"):

        @record
        class WithInitVar:
            transient: dataclasses.InitVar[
                Annotated[int, ("nullable", True)]
            ]

    with pytest.raises(TypeError, match=r"InitVar and ClassVar.*not schema fields"):

        @record
        class WithClassVar:
            shared: ClassVar[Annotated[int, ("arrow_type", pa.int8())]] = 1

    for extra in ({"unit": "items"}, ("unit", "items")):
        with pytest.raises(
            TypeError, match=r"InitVar and ClassVar.*not schema fields"
        ):

            @record
            class WithLegacyMetadata:
                transient: dataclasses.InitVar[Annotated[int, extra]]


def test_parent_struct_nullability_reaches_arrow_output() -> None:
    @record
    class Child:
        value: int

    @record
    class Parent:
        child: Annotated[
            Child,
            (
                "arrow_type",
                pa.struct([pa.field("value", pa.int64(), nullable=True)]),
            ),
        ]

    instance = Parent(Child(None))  # type: ignore[arg-type]
    assert instance.to_dict() == {"child": {"value": None}}
    batch = Parent.into_arrow_record_batch([instance])
    assert batch.column(0)[0].as_py() == {"value": None}


def test_shadow_validation_rejects_nested_non_materialized_options() -> None:
    @dataclasses.dataclass
    class NestedInitVar:
        transient: dataclasses.InitVar[
            Annotated[int, ("nullable", True)]
        ]

    @dataclasses.dataclass
    class NestedClassVar:
        shared: ClassVar[Annotated[int, ("id", 3)]] = 1

    for nested in (NestedInitVar, NestedClassVar):
        with pytest.raises(
            TypeError, match=r"InitVar and ClassVar.*not schema fields"
        ):
            Field.from_pyhint(
                "nested",
                Annotated[nested, ("arrow_type", pa.int64())],
            )


def test_safe_mapping_cast_rejects_post_cast_key_collisions() -> None:
    @record
    class Keyed:
        values: dict[int, int]

    colliding = {"1": 11, 1: 22}
    with pytest.raises(TypeError, match=r"keys\[1\].*collides after safe conversion"):
        Keyed.from_dict({"values": colliding})
    with pytest.raises(TypeError, match=r"keys\[1\].*collides after safe conversion"):
        Keyed(colliding).to_dict()  # type: ignore[arg-type]


def test_explicit_union_requires_exact_logical_branch_alignment() -> None:
    one_child = pa.union(
        [pa.field("integer", pa.int64(), nullable=False)],
        mode="dense",
        type_codes=[0],
    )

    @record
    class Misaligned:
        value: Annotated[int | str, ("arrow_type", one_child)]

    with pytest.raises(
        TypeError,
        match=r"logical union has 2 non-None branches.*has 1 children",
    ):
        Misaligned.from_dict({"value": "text"})

    @record
    class ScalarPhysical:
        value: Annotated[int | str, ("arrow_type", pa.int64())]

    with pytest.raises(TypeError, match=r"physical arrow_type int64 is not a union"):
        ScalarPhysical.from_dict({"value": 1})


def test_annotated_none_union_branch_uses_optional_semantics() -> None:
    nullable_hint = typing.Union[
        int,
        Annotated[type(None), ("note", "null")],
    ]

    @record
    class OptionalValue:
        value: nullable_hint

    assert OptionalValue.from_dict({"value": 1}).value == 1
    assert OptionalValue.from_dict({"value": None}).value is None

    NullNew = typing.NewType("NullNew", type(None))
    maybe_value = typing.TypeVar("maybe_value", type(None), int)
    assert Field.from_pyhint("value", int | NullNew).nullable
    constrained = Field.from_pyhint("value", maybe_value)
    assert constrained.nullable
    assert constrained.data_type.id == "int64"


def test_optional_collapse_promotes_sole_member_field_state() -> None:
    member = Annotated[
        int,
        ("nullable", False),
        ("id", 7),
        ("metadata", {"member": "preserved"}),
    ]
    promoted = Field.from_pyhint("value", typing.Optional[member])
    assert not promoted.nullable
    assert promoted.parquet_field_id == 7
    assert promoted.metadata["member"] == "preserved"

    identity_override = Field.from_pyhint(
        "value",
        typing.Optional[
            Annotated[int, ("metadata", {"python.class": "custom"})]
        ],
    )
    assert identity_override.metadata["python.class"] == "custom"

    outer = Field.from_pyhint(
        "value",
        Annotated[
            typing.Optional[member],
            ("nullable", True),
            ("id", 8),
            ("metadata", {"member": "outer"}),
        ],
    )
    assert outer.nullable
    assert outer.parquet_field_id == 8
    assert outer.metadata["member"] == "outer"

    constrained = typing.TypeVar(
        "constrained",
        member,
        type(None),
    )
    promoted_constraint = Field.from_pyhint("value", constrained)
    assert not promoted_constraint.nullable
    assert promoted_constraint.parquet_field_id == 7
    assert promoted_constraint.metadata["member"] == "preserved"

    bound = typing.TypeVar("bound", bound=typing.Optional[member])
    promoted_bound = Field.from_pyhint("value", bound)
    assert not promoted_bound.nullable
    assert promoted_bound.parquet_field_id == 7

    wrapped = typing.NewType("wrapped", typing.Optional[member])
    promoted_newtype = Field.from_pyhint("value", wrapped)
    assert not promoted_newtype.nullable
    assert promoted_newtype.parquet_field_id == 7

    plain_bound = typing.TypeVar("plain_bound", bound=member)
    assert Field.from_pyhint("value", plain_bound).parquet_field_id == 7
    plain_newtype = typing.NewType("plain_newtype", member)
    assert Field.from_pyhint("value", plain_newtype).parquet_field_id == 7


def test_plain_class_classvar_options_do_not_disappear() -> None:
    class Plain:
        shared: ClassVar[Annotated[int, ("id", 3)]] = 1
        value: int

    with pytest.raises(TypeError, match=r"ClassVar.*not schema fields"):
        Field.from_pyhint("plain", Plain)
    with pytest.raises(TypeError, match=r"ClassVar.*not schema fields"):
        Field.from_pyhint(
            "plain",
            Annotated[Plain, ("arrow_type", pa.int64())],
        )

    hidden_newtype = typing.NewType(
        "hidden_newtype", Annotated[int, ("id", 5)]
    )
    hidden_typevar = typing.TypeVar(
        "hidden_typevar",
        bound=Annotated[int, {"unit": "items"}],
    )
    nested = list[Annotated[int, ("nullable", True)]]
    for hidden in (hidden_newtype, hidden_typevar, nested):
        class Hidden:
            value: int

        Hidden.__annotations__["shared"] = ClassVar[hidden]
        with pytest.raises(TypeError, match=r"ClassVar.*not schema fields"):
            Field.from_pyhint("hidden", Hidden)


def test_defaults_cannot_bypass_explicit_nullability() -> None:
    @record
    class InvalidDefault:
        value: Annotated[int | None, ("nullable", False)] = None

    with pytest.raises(TypeError, match=r"value.*not nullable"):
        InvalidDefault.from_dict({})
    with pytest.raises(TypeError, match=r"value.*not nullable"):
        InvalidDefault.from_dict({"value": "bad"}, errors="default")

    calls = 0

    def invalid_factory() -> None:
        nonlocal calls
        calls += 1
        return None

    @record
    class InvalidFactory:
        value: Annotated[int, ("nullable", False)] = dataclasses.field(
            default_factory=invalid_factory
        )

    with pytest.raises(TypeError, match=r"value.*not nullable"):
        InvalidFactory.from_dict({})
    assert calls == 1

    @record
    class InvalidInitFalse:
        value: Annotated[int, ("nullable", False)] = dataclasses.field(
            init=False, default=None
        )

    with pytest.raises(TypeError, match=r"value.*not nullable"):
        InvalidInitFalse.from_dict({})


def test_pep695_alias_unions_resolve_for_safe_output() -> None:
    Maybe = typing_extensions.TypeAliasType("Maybe", int | None)
    Either = typing_extensions.TypeAliasType("Either", int | str)
    ValueT = typing.TypeVar("ValueT")
    GenericMaybe = typing_extensions.TypeAliasType(
        "GenericMaybe",
        ValueT | None,
        type_params=(ValueT,),
    )

    @record
    class Aliases:
        maybe: Maybe
        either: Either
        generic: GenericMaybe[int]

    instance = Aliases(1, "text", 2)
    assert instance.to_dict() == {
        "maybe": 1,
        "either": "text",
        "generic": 2,
    }

    @record
    class OptionalAliases:
        maybe: Maybe
        generic: GenericMaybe[int]

    batch = OptionalAliases.into_arrow_record_batch(
        [OptionalAliases(1, 2)]
    )
    assert batch.to_pydict() == {"maybe": [1], "generic": [2]}


def test_safe_physical_list_and_temporal_values_are_lossless() -> None:
    fixed = pa.list_(pa.int16(), 2)

    @record
    class Physical:
        values: Annotated[list[int], ("arrow_type", fixed)]
        instant: Annotated[
            dt.datetime,
            ("arrow_type", pa.timestamp("ms", tz="UTC")),
        ]
        clock: Annotated[dt.time, ("arrow_type", pa.time32("s"))]
        elapsed: Annotated[
            dt.timedelta,
            ("arrow_type", pa.duration("ms")),
        ]

    valid_values = {
        "values": [1, 2],
        "instant": dt.datetime(
            2026, 8, 15, 12, 30, 1, 123_000, tzinfo=dt.timezone.utc
        ),
        "clock": dt.time(12, 30, 1),
        "elapsed": dt.timedelta(seconds=1, milliseconds=234),
    }
    valid = Physical.from_dict(valid_values)
    assert valid.to_dict() == valid_values
    assert Physical.into_arrow_record_batch([valid]).num_rows == 1

    invalid = dict(valid_values)
    invalid["values"] = [1]
    with pytest.raises(TypeError, match=r"values.*exactly 2 items"):
        Physical.from_dict(invalid)
    with pytest.raises(TypeError, match=r"values.*exactly 2 items"):
        Physical(
            [1],
            valid.instant,
            valid.clock,
            valid.elapsed,
        ).to_dict()

    invalid = dict(valid_values)
    invalid["instant"] = dt.datetime(
        2026, 8, 15, 12, 30, 1, 123_456, tzinfo=dt.timezone.utc
    )
    with pytest.raises(TypeError, match=r"instant.*would truncate"):
        Physical.from_dict(invalid)

    invalid["instant"] = dt.datetime(2026, 8, 15, 12, 30, 1, 123_000)
    assert Physical.from_dict(invalid).instant == invalid["instant"]

    class NanosecondDateTime(dt.datetime):
        @property
        def nanosecond(self) -> int:
            return 1

    invalid["instant"] = NanosecondDateTime(
        2026, 8, 15, 12, 30, 1, 123_000, tzinfo=dt.timezone.utc
    )
    with pytest.raises(TypeError, match=r"instant.*would truncate"):
        Physical.from_dict(invalid)

    invalid = dict(valid_values)
    invalid["clock"] = dt.time(12, 30, 1, 1)
    with pytest.raises(TypeError, match=r"clock.*would truncate"):
        Physical.from_dict(invalid)
    invalid["clock"] = dt.time(12, 30, tzinfo=dt.timezone.utc)
    with pytest.raises(TypeError, match=r"clock.*timezone-aware time"):
        Physical.from_dict(invalid)

    invalid = dict(valid_values)
    invalid["elapsed"] = dt.timedelta(microseconds=1_234)
    with pytest.raises(TypeError, match=r"elapsed.*would truncate"):
        Physical.from_dict(invalid)

    class NanosecondDelta(dt.timedelta):
        @property
        def nanoseconds(self) -> int:
            return 1

    invalid["elapsed"] = NanosecondDelta(milliseconds=1)
    with pytest.raises(TypeError, match=r"elapsed.*would truncate"):
        Physical.from_dict(invalid)

    @record
    class LocalTimestamp:
        instant: Annotated[
            dt.datetime,
            ("arrow_type", pa.timestamp("us")),
        ]

    with pytest.raises(TypeError, match=r"timezone-aware datetime"):
        LocalTimestamp.from_dict(
            {"instant": dt.datetime(2026, 8, 15, tzinfo=dt.timezone.utc)}
        )

    @record
    class ParisTimestamp:
        instant: Annotated[
            dt.datetime,
            ("arrow_type", pa.timestamp("us", tz="Europe/Paris")),
        ]

    with pytest.raises(TypeError, match=r"instant.*naive datetime"):
        ParisTimestamp.from_dict({"instant": dt.datetime(2026, 8, 15)})

    @record
    class SecondTimestamp:
        instant: Annotated[
            dt.datetime,
            ("arrow_type", pa.timestamp("s", tz="UTC")),
        ]

    subsecond_offset = dt.timezone(dt.timedelta(microseconds=1))
    lossy_instant = dt.datetime(
        2026, 8, 15, 12, 30, tzinfo=subsecond_offset
    )
    with pytest.raises(TypeError, match=r"instant.*would truncate"):
        SecondTimestamp.from_dict({"instant": lossy_instant})
    with pytest.raises(TypeError, match=r"instant.*would truncate"):
        SecondTimestamp(lossy_instant).to_dict()

    whole_second_offset = dt.timezone(dt.timedelta(seconds=30))
    exact_instant = dt.datetime(
        2026, 8, 15, 12, 30, tzinfo=whole_second_offset
    )
    assert SecondTimestamp.from_dict({"instant": exact_instant}).instant == (
        exact_instant
    )
