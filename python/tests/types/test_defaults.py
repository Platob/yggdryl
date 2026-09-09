from __future__ import annotations

import dataclasses
import datetime as dt
import typing
from decimal import Decimal

import pyarrow as pa
import pytest

from yggdryl import DataType, Field, Scalar, Version, types

# Every Arrow datatype variant the core distinguishes. It is asserted as a
# constant so that adding a variant to the core without adding it here fails,
# independently of how many of them the installed PyArrow can spell.
CATALOGUE_SIZE = 41


def _available(*builders: typing.Callable[[], pa.DataType]) -> tuple[pa.DataType, ...]:
    """Build every variant this PyArrow can spell, skipping the rest.

    The package floor is PyArrow 18, which predates the narrow decimals that
    arrived in 19. Those are absent rather than broken there, so a release
    that has no constructor for one runs the others instead of failing to
    collect the module.
    """
    built = []
    for build in builders:
        try:
            built.append(build())
        except AttributeError:
            continue
    return tuple(built)


def _all_datatype_variants() -> tuple[DataType, ...]:
    item = pa.field("item", pa.int32(), nullable=True)
    # Each variant is built by a callable rather than eagerly, because the
    # package floor is PyArrow 18 and the narrow-decimal constructors arrived
    # in 19. A release that cannot spell one simply does not run it;
    # `CATALOGUE_SIZE` is what keeps the catalogue itself complete.
    arrow_types = _available(
        lambda: pa.null(),
        lambda: pa.bool_(),
        lambda: pa.int8(),
        lambda: pa.int16(),
        lambda: pa.int32(),
        lambda: pa.int64(),
        lambda: pa.uint8(),
        lambda: pa.uint16(),
        lambda: pa.uint32(),
        lambda: pa.uint64(),
        lambda: pa.float16(),
        lambda: pa.float32(),
        lambda: pa.float64(),
        lambda: pa.timestamp("ns", tz="UTC"),
        lambda: pa.date32(),
        lambda: pa.date64(),
        lambda: pa.time32("ms"),
        lambda: pa.time64("ns"),
        lambda: pa.duration("us"),
        lambda: pa.month_day_nano_interval(),
        lambda: pa.binary(),
        lambda: pa.binary(3),
        lambda: pa.large_binary(),
        lambda: pa.binary_view(),
        lambda: pa.string(),
        lambda: pa.large_string(),
        lambda: pa.string_view(),
        lambda: pa.list_(item),
        lambda: pa.list_view(item),
        lambda: pa.list_(item, 2),
        lambda: pa.large_list(item),
        lambda: pa.large_list_view(item),
        lambda: pa.struct(
            [
                pa.field("required", pa.int32(), nullable=False),
                pa.field("optional", pa.string(), nullable=True),
            ]
        ),
        lambda: pa.dense_union(
            [
                pa.field("missing", pa.null(), nullable=True),
                pa.field("number", pa.int32(), nullable=False),
            ],
            type_codes=[1, 4],
        ),
        lambda: pa.dictionary(pa.int8(), pa.string()),
        lambda: pa.decimal32(7, 2),
        lambda: pa.decimal64(12, 2),
        lambda: pa.decimal128(30, 2),
        lambda: pa.decimal256(50, 2),
        lambda: pa.map_(pa.string(), pa.int32()),
        lambda: pa.run_end_encoded(pa.int32(), pa.string()),
    )
    variants = tuple(DataType.from_arrow(dtype) for dtype in arrow_types)
    # `id` is the per-variant identity; `kind` is the coarse family, so the
    # variants collapse into far fewer kinds.
    assert len({dtype.id for dtype in variants}) == len(variants)
    assert len({dtype.kind for dtype in variants}) < len(variants)
    return variants


def test_the_catalogue_names_every_variant_this_pyarrow_can_spell() -> None:
    built = len(_all_datatype_variants())

    # A PyArrow new enough to spell them all must spell all of them; an older
    # one says how many it left out rather than quietly testing fewer.
    if hasattr(pa, "binary_view") and hasattr(pa, "decimal32"):
        assert built == CATALOGUE_SIZE
    else:
        assert built < CATALOGUE_SIZE, f"{built} of {CATALOGUE_SIZE} variants"


@pytest.mark.parametrize(
    "dtype",
    _all_datatype_variants(),
    ids=lambda dtype: dtype.id,
)
def test_every_datatype_variant_has_one_python_and_arrow_default(
    dtype: DataType,
) -> None:
    scalar = dtype.default_arrow_scalar()

    assert scalar.type.equals(dtype.into_arrow())
    dtype.default_pyhint()
    assert isinstance(dtype.default_scalar(), Scalar)
    dtype.default_scalar().as_py()


def test_a_default_is_one_scalar_that_materializes_the_same_value() -> None:
    required = Field(
        "row",
        DataType.from_fields(
            (
                Field("count", "uint32", nullable=False),
                Field("label", "utf8", nullable=True),
            )
        ),
        nullable=False,
    )
    nullable = Field("row", required.dtype, nullable=True)

    default = required.default_scalar()

    assert isinstance(default, Scalar)
    assert default.as_py() == [0, None]
    # The materialized default is that same Scalar rather than a second
    # answer: rendered into Arrow it reproduces `default_arrow_scalar`
    # exactly, and a required cast fills its nulls with it.
    assert default.into_arrow_scalar(required).equals(required.default_arrow_scalar())
    filled = required.dtype.cast_arrow_array(
        pa.nulls(2, required.dtype.into_arrow())
    )
    assert filled.to_pylist() == [{"count": 0, "label": None}] * 2

    # Nullability is the core's answer over one layout: the datatype answers
    # its canonical value, and a Field that may be absent answers null.
    assert required.dtype.default_scalar() == default
    assert nullable.default_scalar().kind == "null"
    assert nullable.default_scalar().as_py() is None


def test_default_pyhint_is_cached_nullable_and_arrow_free() -> None:
    import yggdryl.types._arrow as field_arrow

    assert not hasattr(field_arrow, "_pyarrow")
    nested = DataType.from_fields(
        (
            Field("identifier", "uint32", nullable=False),
            Field(
                "child",
                DataType.from_fields(
                    (Field("label", "utf8", nullable=False),)
                ),
                nullable=False,
            ),
        )
    )

    hint = nested.default_pyhint()
    assert hint is nested.default_pyhint()
    assert dataclasses.is_dataclass(hint)
    assert hint.__annotations__["identifier"] is int
    assert dataclasses.is_dataclass(hint.__annotations__["child"])

    left = Field("left", "int32", metadata={"unit": "first"})
    right = Field("right", "int32", metadata={"unit": "second"})
    nullable_hint = left.default_pyhint()
    assert nullable_hint is right.default_pyhint()
    assert set(typing.get_args(nullable_hint)) == {int, type(None)}
    assert Field("required", "int32", nullable=False).default_pyhint() is int


@pytest.mark.parametrize("first", ["left", "right"])
def test_default_pyhint_cache_ignores_nested_metadata_recursively(
    first: str,
) -> None:
    def nested(owner: str) -> DataType:
        return DataType.from_fields(
            (
                Field(
                    "payload",
                    DataType.from_fields(
                        (
                            Field(
                                "count",
                                "int32",
                                nullable=False,
                                metadata={"leaf-owner": owner},
                            ),
                        )
                    ),
                    nullable=False,
                    metadata={"struct-owner": owner},
                ),
            )
        )

    layouts = {"left": nested("left"), "right": nested("right")}
    left = layouts["left"]
    right = layouts["right"]
    assert left != right
    assert left.equals(right, with_metadata=False)

    second = "right" if first == "left" else "left"
    hint = layouts[first].default_pyhint()
    assert layouts[second].default_pyhint() is hint

    payload_hint = hint.__annotations__["payload"]
    assert payload_hint.field().metadata.get("struct-owner") is None
    assert (
        payload_hint.field()
        .dtype["count"]
        .metadata.get("leaf-owner")
        is None
    )


def test_typed_factory_defaults_cover_field_and_nested_child_nullability() -> None:
    nullable_item = types.int32("item")
    required_item = types.int32("item", nullable=False)

    assert nullable_item.default_scalar().as_py() is None
    assert required_item.default_scalar().as_py() == 0
    assert required_item.dtype.default_scalar().as_py() == 0

    fixed = types.fixed_size_list(
        "values", nullable_item, 2, nullable=False
    )
    assert fixed.default_scalar().as_py() == [None, None]
    assert fixed.dtype.default_scalar().as_py() == [None, None]

    valid_struct = types.struct(
        "row", (required_item,), nullable=False
    )
    invalid_struct = types.struct(
        "row", (Field("not-valid", "int32", nullable=False),), nullable=False
    )
    # A record default is positional, so both spell the same value; only the
    # hint splits on whether the child names are Python identifiers.
    assert valid_struct.default_scalar().as_py() == [0]
    assert invalid_struct.default_scalar().as_py() == [0]
    assert dataclasses.is_dataclass(valid_struct.default_pyhint())
    assert typing.is_typeddict(invalid_struct.default_pyhint())


def test_struct_field_metadata_never_reaches_the_cached_hint() -> None:
    dtype = DataType.from_fields(
        (Field("count", "int32", nullable=False),)
    )
    left = Field("left", dtype, nullable=False, metadata={"owner": "left"})
    right = Field("right", dtype, nullable=False, metadata={"owner": "right"})

    hint = left.default_pyhint()

    # Name and metadata are not part of hint identity, so two Fields over one
    # layout share a single hint whose own Field carries neither of theirs.
    assert right.default_pyhint() is hint
    assert hint.field().metadata.get("owner") is None
    assert hint.field().name not in ("left", "right")


def test_non_identifier_struct_names_use_typed_mapping_fallback() -> None:
    arrow_type = pa.struct(
        [
            pa.field("a-b", pa.int16(), nullable=False),
            pa.field("class", pa.decimal128(9, 2), nullable=True),
            pa.field(
                "1child",
                pa.struct([pa.field("label", pa.string(), nullable=False)]),
                nullable=False,
                metadata={b"role": b"nested"},
            ),
        ]
    )
    dtype = DataType.from_arrow(arrow_type)

    hint = dtype.default_pyhint()

    assert typing.is_typeddict(hint)
    assert tuple(hint.__annotations__) == ("a-b", "class", "1child")
    nested_hint = hint.__annotations__["1child"]
    assert nested_hint.field().metadata.get("role") is None
    # The fallback names the hint's keys only: the default stays positional.
    assert dtype.default_scalar().as_py() == [0, None, [""]]
    assert dtype.default_arrow_scalar().type.equals(arrow_type)


def test_a_default_scalar_reads_as_generic_python_values() -> None:
    dtype = DataType.from_arrow(
        pa.struct(
            [
                pa.field("identifier", pa.uint32(), nullable=False),
                pa.field("amount", pa.decimal128(18, 4), nullable=False),
                pa.field("optional", pa.string(), nullable=True),
                pa.field("day", pa.date32(), nullable=False),
                pa.field("tags", pa.map_(pa.string(), pa.int32()), nullable=False),
                pa.field("blob", pa.binary(2), nullable=False),
                pa.field(
                    "child",
                    pa.struct([pa.field("label", pa.string(), nullable=False)]),
                    nullable=False,
                ),
            ]
        )
    )

    # A record is a positional list and a map is a dict; nothing here is a
    # generated class, and the hint alone names the children.
    assert dtype.default_scalar().as_py() == [
        0,
        Decimal("0.0000"),
        None,
        dt.date(1970, 1, 1),
        {},
        b"\x00\x00",
        [""],
    ]
    assert Field("id", "uuid", nullable=False).default_scalar().as_py() == (
        "00000000-0000-0000-0000-000000000000"
    )
    assert Field("release", "version", nullable=False).default_scalar().as_py() == Version(0)
    assert DataType.ascii(3).default_scalar().as_py() == ""


@pytest.mark.parametrize(
    ("arrow_type", "expected"),
    [
        (pa.decimal256(39, 4), Decimal("0.0000")),
        (
            pa.timestamp("us", tz="Europe/Paris"),
            dt.datetime(1970, 1, 1, 1, tzinfo=dt.timezone(dt.timedelta(hours=1))),
        ),
        (pa.date32(), dt.date(1970, 1, 1)),
        (pa.time64("us"), dt.time()),
        (pa.duration("ns"), dt.timedelta()),
        (pa.binary(4), b"\x00\x00\x00\x00"),
        (pa.list_(pa.int16()), []),
    ],
)
def test_scalar_defaults_share_exact_arrow_and_python_projection(
    arrow_type: pa.DataType, expected: object
) -> None:
    dtype = DataType.from_arrow(arrow_type)
    scalar = dtype.default_arrow_scalar()

    assert scalar.type.equals(arrow_type)
    assert scalar.as_py() == expected
    assert dtype.default_scalar().as_py() == expected


def test_field_default_nullability_comes_from_native_core() -> None:
    nullable = Field("amount", DataType.decimal(18, 4), nullable=True)
    required = Field("amount", DataType.decimal(18, 4), nullable=False)

    assert nullable.default_scalar().as_py() is None
    assert required.default_scalar().as_py() == Decimal("0.0000")

    # `decimal(18, 4)` is a decimal64, whose Arrow type id a PyArrow below 19
    # cannot import at all. The Scalar never crosses that boundary, so only
    # the Arrow half runs where the release can spell one.
    if hasattr(pa, "decimal64"):
        nullable_scalar = nullable.default_arrow_scalar()
        required_scalar = required.default_arrow_scalar()
        assert nullable_scalar.type == pa.decimal64(18, 4)
        assert not nullable_scalar.is_valid
        assert required_scalar.as_py() == Decimal("0.0000")

    null_type = DataType("null")
    assert not null_type.default_arrow_scalar().is_valid
    assert null_type.default_scalar().as_py() is None
    impossible = Field("impossible", null_type, nullable=False)
    with pytest.raises(ValueError, match="non-nullable|no constructible|default"):
        impossible.default_arrow_scalar()
    with pytest.raises(ValueError, match="non-nullable|no constructible|default"):
        impossible.default_scalar()


def test_nullable_struct_default_masks_uninhabited_nested_physical_children() -> None:
    inner = Field(
        "inner",
        DataType.from_fields(
            (Field("required_null", "null", nullable=False),)
        ),
        nullable=False,
    )
    outer = Field(
        "outer",
        DataType.from_fields((inner,)),
        nullable=True,
    )

    scalar = outer.default_arrow_scalar()
    assert outer.default_scalar().kind == "null"
    assert outer.default_scalar().as_py() is None
    assert scalar.type.equals(outer.dtype.into_arrow())
    assert not scalar.is_valid
    assert scalar.as_py() is None


def test_fixed_struct_union_dictionary_and_run_end_defaults() -> None:
    item_type = pa.struct(
        [
            pa.field("code", pa.int16(), nullable=False),
            pa.field("note", pa.string(), nullable=True),
        ]
    )
    fixed = DataType.from_arrow(
        pa.list_(pa.field("item", item_type, nullable=False), 2)
    )
    assert fixed.default_scalar().as_py() == [[0, None], [0, None]]

    union_type = pa.dense_union(
        [
            pa.field("missing", pa.null(), nullable=True),
            pa.field("count", pa.int32(), nullable=False),
        ],
        type_codes=[3, 7],
    )
    union = DataType.from_arrow(union_type)
    union_scalar = union.default_arrow_scalar()
    assert union_scalar.type.equals(union_type)
    assert union_scalar.type_code == 7
    # A union Scalar spells the selected branch beside its value, so the code
    # the Arrow scalar carries is the head of the pair.
    assert union.default_scalar().as_py() == [7, 0]

    dictionary_type = pa.dictionary(pa.int8(), pa.string(), ordered=True)
    dictionary = DataType.from_arrow(dictionary_type)
    dictionary_scalar = dictionary.default_arrow_scalar()
    assert dictionary_scalar.type.equals(dictionary.into_arrow())
    assert dictionary_scalar.as_py() == ""
    assert dictionary.default_pyhint() is str
    assert dictionary.default_scalar().as_py() == ""
    ordered_dictionary = Field.from_arrow(
        pa.field("ordered", dictionary_type, nullable=False)
    )
    unchanged = Field.from_value(ordered_dictionary)
    assert ordered_dictionary.default_arrow_scalar().type.equals(dictionary_type)
    assert ordered_dictionary == unchanged

    run_end_type = pa.run_end_encoded(pa.int16(), pa.int64())
    run_end = DataType.from_arrow(run_end_type)
    run_end_scalar = run_end.default_arrow_scalar()
    assert run_end_scalar.type.equals(run_end_type)
    assert run_end.default_pyhint() is int
    assert run_end.default_scalar().as_py() == 0


def test_variant_defaults_retain_collapsed_physical_branch_selection() -> None:
    duplicate_python_hint = DataType.variant(
        (
            Field("narrow", "int32", nullable=True),
            Field("wide", "int64", nullable=True),
        )
    )
    assert set(typing.get_args(duplicate_python_hint.default_pyhint())) == {
        int,
        type(None),
    }
    assert duplicate_python_hint.default_scalar().as_py() == [0, 0]

    nullable_choice = Field("choice", duplicate_python_hint, nullable=True)
    nullable_scalar = nullable_choice.default_arrow_scalar()
    assert nullable_scalar.type_code == 0
    assert not nullable_scalar.is_valid
    # The branch survives the absence: a nullable choice is the first branch
    # holding null, not a bare null.
    assert nullable_choice.default_scalar().as_py() == [0, None]

    nullable_nested = DataType.from_fields((nullable_choice,))
    assert nullable_nested.default_scalar().as_py() == [[0, None]]

    impossible = Field(
        "fixed",
        DataType._list(
            "fixed_size_list",
            Field("item", "null", nullable=False),
            1,
        ),
        nullable=False,
    )
    selected = Field(
        "variable",
        DataType._list("list", Field("item", "null")),
        nullable=False,
    )
    selected_second = DataType.variant((impossible, selected))
    assert selected_second.default_arrow_scalar().type_code == 1
    assert selected_second.default_scalar().as_py() == [1, []]

    uninhabited_struct = DataType.from_fields(
        (Field("required", "null", nullable=False),)
    )
    present_struct = DataType.from_fields(
        (Field("value", "int32", nullable=False),)
    )
    structured = DataType.variant(
        (
            Field("missing", uninhabited_struct, nullable=False),
            Field(
                "present",
                present_struct,
                nullable=False,
                metadata={"branch": "selected"},
            ),
        )
    )
    assert structured.default_arrow_scalar().type_code == 1
    assert structured.default_scalar().as_py() == [1, [0]]

    nested = DataType.from_fields(
        (
            Field("choice", duplicate_python_hint, nullable=False),
            Field(
                "repeated",
                DataType._list(
                    "fixed_size_list",
                    Field("item", duplicate_python_hint, nullable=False),
                    2,
                ),
                nullable=False,
            ),
        )
    )
    assert nested.default_scalar().as_py() == [[0, 0], [[0, 0], [0, 0]]]


def test_default_arrow_scalar_rehydrates_registered_extension() -> None:
    class DefaultExtension(pa.ExtensionType):
        def __init__(self) -> None:
            super().__init__(pa.int32(), "tests.defaults.extension")

        def __arrow_ext_serialize__(self) -> bytes:
            return b"v1"

        @classmethod
        def __arrow_ext_deserialize__(
            cls, storage_type: pa.DataType, serialized: bytes
        ) -> DefaultExtension:
            assert storage_type == pa.int32()
            assert serialized == b"v1"
            return cls()

    extension = DefaultExtension()
    pa.register_extension_type(extension)
    try:
        required = Field.from_arrow(pa.field("value", extension, nullable=False))
        nullable = Field.from_arrow(pa.field("value", extension, nullable=True))

        present = required.default_arrow_scalar()
        missing = nullable.default_arrow_scalar()
        assert present.type.equals(extension)
        assert present.as_py() == 0
        assert required.default_scalar().as_py() == 0
        assert missing.type.equals(extension)
        assert not missing.is_valid
        assert nullable.default_scalar().as_py() is None
    finally:
        pa.unregister_extension_type(extension.extension_name)


def test_struct_extension_default_keeps_its_storage_and_metadata_free_hint() -> None:
    class StructExtension(pa.ExtensionType):
        def __init__(self) -> None:
            super().__init__(
                pa.struct([pa.field("count", pa.int32(), nullable=False)]),
                "tests.defaults.struct-extension",
            )

        def __arrow_ext_serialize__(self) -> bytes:
            return b"struct-v1"

        @classmethod
        def __arrow_ext_deserialize__(
            cls, storage_type: pa.DataType, serialized: bytes
        ) -> StructExtension:
            assert storage_type == pa.struct(
                [pa.field("count", pa.int32(), nullable=False)]
            )
            assert serialized == b"struct-v1"
            return cls()

    extension = StructExtension()
    pa.register_extension_type(extension)
    try:
        field = Field.from_arrow(
            pa.field(
                "payload",
                extension,
                nullable=False,
                metadata={b"owner": b"tests"},
            )
        )
        hint = field.default_pyhint()
        scalar = field.default_arrow_scalar()

        assert scalar.type.equals(extension)
        assert scalar.as_py() == {"count": 0}
        assert field.default_scalar().as_py() == [0]
        assert dataclasses.is_dataclass(hint)
        assert hint.field().metadata.get("owner") is None
    finally:
        pa.unregister_extension_type(extension.extension_name)


def test_scheme_compatibility_is_native_recursive_and_typed() -> None:
    source = Field(
        "root",
        DataType.from_fields(
            (
                Field("small", "uint8", nullable=False),
                Field("wide", "uint64", nullable=False),
                Field("half", "float16", nullable=False),
            )
        ),
        nullable=False,
        metadata={"owner": "tests"},
    )

    assert source.into_scheme_compat("arrow") == source
    spark = source.into_scheme_compat("spark")
    assert spark.name == "root"
    assert spark.nullable is False
    assert dict(spark.metadata.items()) == {"owner": "tests"}
    assert [field.dtype.id for field in spark.dtype] == [
        "int16",
        "decimal128",
        "float32",
    ]
    assert spark.dtype["wide"].dtype == DataType.decimal(20, 0)

    with pytest.raises(ValueError, match="Spark|spark|microsecond"):
        DataType.from_arrow(pa.timestamp("ns")).into_scheme_compat("spark")
    with pytest.raises(ValueError):
        source.into_scheme_compat("parquet")  # type: ignore[arg-type]
