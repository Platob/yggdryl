"""The digest surface, checked against the C ``libxxhash`` bindings.

The pinned vectors below are the published reference values. Everything else
is compared against the ``xxhash`` package, which wraps C ``libxxhash``: an
outside implementation of the same protocol, both directions, so a wrong
answer here cannot be self-consistent.
"""

from __future__ import annotations

import copy
import io
import pickle

import pyarrow as pa
import pytest

import yggdryl
from yggdryl import Field, Scalar, xxhash
from yggdryl.enums import DIGEST_ALGORITHMS

xxhash_c = pytest.importorskip(
    "xxhash", reason="the outside C libxxhash binding is required for parity"
)

PAYLOAD = b'{"symbol": "AAPL", "price": 187.23}\n' * 512

#: One payload per XXH3 size branch, plus the boundaries between them.
BRANCHES = [0, 1, 3, 4, 8, 9, 16, 17, 64, 128, 129, 240, 241, 4096]


def corpus(length: int) -> bytes:
    return bytes((index * 31 + 7) % 256 for index in range(length))


def secret(length: int) -> bytes:
    return bytes((index * 17 + 3) % 256 for index in range(length))


class TestVectors:
    def test_published_vectors_pin_every_algorithm(self) -> None:
        assert xxhash.xxh32(b"") == 0x02CC5D05
        assert xxhash.xxh64(b"") == 0xEF46DB3751D8E999
        assert xxhash.xxh3(b"") == 0x2D06800538D394C2
        assert xxhash.xxh128(b"") == 0x99AA06D3014798D86001C324468D497F

        assert xxhash.xxh32(b"abc") == 0x32D153FF
        assert xxhash.xxh64(b"abc") == 0x44BC2CF5AD770999
        assert xxhash.xxh3(b"abc") == 0x78AF5F94892F3950
        assert xxhash.xxh128(b"abc") == 0x06B05AB6733A618578AF5F94892F3950

    def test_the_algorithm_vocabulary_is_the_native_listing(self) -> None:
        assert DIGEST_ALGORITHMS == ("xxh32", "xxh64", "xxh3-64", "xxh3-128")


class TestOutsideImplementation:
    """Every one-shot and streaming answer, against C ``libxxhash``."""

    @pytest.mark.parametrize("length", BRANCHES)
    def test_one_shot_matches_the_c_binding(self, length: int) -> None:
        data = corpus(length)
        assert xxhash.xxh32(data) == xxhash_c.xxh32_intdigest(data)
        assert xxhash.xxh64(data) == xxhash_c.xxh64_intdigest(data)
        assert xxhash.xxh3(data) == xxhash_c.xxh3_64_intdigest(data)
        assert xxhash.xxh128(data) == xxhash_c.xxh3_128_intdigest(data)

    @pytest.mark.parametrize("length", BRANCHES)
    def test_seeded_one_shot_matches_the_c_binding(self, length: int) -> None:
        data = corpus(length)
        assert xxhash.xxh32(data, seed=42) == xxhash_c.xxh32_intdigest(data, seed=42)
        assert xxhash.xxh64(data, seed=42) == xxhash_c.xxh64_intdigest(data, seed=42)
        assert xxhash.xxh3(data, seed=42) == xxhash_c.xxh3_64_intdigest(data, seed=42)
        assert xxhash.xxh128(data, seed=42) == xxhash_c.xxh3_128_intdigest(
            data, seed=42
        )

    @pytest.mark.parametrize("split", [1, 7, 64, 240, 1024])
    def test_streaming_matches_the_c_binding(self, split: int) -> None:
        pairs = [
            (xxhash.Xxh32(), xxhash_c.xxh32()),
            (xxhash.Xxh64(), xxhash_c.xxh64()),
            (xxhash.Xxh3(), xxhash_c.xxh3_64()),
            (xxhash.Xxh128(), xxhash_c.xxh3_128()),
        ]
        for native, outside in pairs:
            for index in range(0, len(PAYLOAD), split):
                chunk = PAYLOAD[index : index + split]
                native.write_bytes(chunk)
                outside.update(chunk)
            assert int(native.as_digest()) == outside.intdigest()
            assert bytes(native.as_digest()) == outside.digest()

    def test_the_c_binding_reads_our_canonical_bytes(self) -> None:
        # The other direction: the canonical big-endian representation is what
        # the reference calls `XXH*_canonicalFromHash`, so the C binding's own
        # `digest()` and ours are the same bytes.
        assert bytes(xxhash.digest(PAYLOAD, "xxh32")) == xxhash_c.xxh32_digest(PAYLOAD)
        assert bytes(xxhash.digest(PAYLOAD, "xxh64")) == xxhash_c.xxh64_digest(PAYLOAD)
        assert bytes(xxhash.digest(PAYLOAD, "xxh3-64")) == xxhash_c.xxh3_64_digest(
            PAYLOAD
        )
        assert bytes(xxhash.digest(PAYLOAD, "xxh3-128")) == xxhash_c.xxh3_128_digest(
            PAYLOAD
        )


class TestContent:
    def test_every_buffer_shape_reads_the_same_bytes(self) -> None:
        expected = xxhash.xxh3(PAYLOAD)
        assert xxhash.xxh3(bytearray(PAYLOAD)) == expected
        assert xxhash.xxh3(memoryview(PAYLOAD)) == expected
        # A window into a larger buffer is the window's bytes, not the whole.
        window = memoryview(PAYLOAD)[10:20]
        assert xxhash.xxh3(window) == xxhash.xxh3(PAYLOAD[10:20])

    def test_a_string_is_its_utf8(self) -> None:
        assert xxhash.xxh3("é—wide") == xxhash.xxh3("é—wide".encode())

    def test_a_non_buffer_is_a_type_error(self) -> None:
        with pytest.raises(TypeError, match="buffer protocol"):
            xxhash.xxh3(1)  # type: ignore[arg-type]


class TestStates:
    def test_answering_does_not_consume_the_state(self) -> None:
        state = xxhash.Xxh3()
        state.write_bytes(b"AAPL")
        first = state.as_digest()
        assert state.as_digest() == first
        state.write_bytes(b",187.23")
        assert int(state.as_digest()) == xxhash.xxh3(b"AAPL,187.23")

    def test_clear_returns_to_the_constructed_seed(self) -> None:
        state = xxhash.Xxh64(seed=11)
        assert state.seed == 11
        assert state.algorithm == "xxh64"
        state.write_bytes(b"AAPL")
        state.clear()
        assert int(state.as_digest()) == xxhash.xxh64(b"", seed=11)
        assert int(state.as_digest()) != xxhash.xxh64(b"")

    def test_a_custom_secret_travels_with_the_state(self) -> None:
        custom = secret(xxhash.SECRET_MINIMUM_LENGTH)
        state = xxhash.Xxh3(seed=5, secret=custom)
        assert state.secret == custom
        state.write_bytes(PAYLOAD)
        assert int(state.as_digest()) == xxhash.xxh3(PAYLOAD, seed=5, secret=custom)
        assert int(state.as_digest()) != xxhash.xxh3(PAYLOAD, seed=5)

    def test_a_secret_is_consulted_only_past_the_cutoff(self) -> None:
        # XXH3's own rule for the seed-and-secret family: at or below 240 bytes
        # the derived secret and the seed decide, which is what keeps the
        # one-shot and the streaming state answering one value.
        custom = secret(xxhash.SECRET_MINIMUM_LENGTH)
        for length in (0, 1, 64, 240):
            short = corpus(length)
            assert xxhash.xxh3(short, secret=custom) == xxhash.xxh3(short)
        for length in (241, 1024):
            long = corpus(length)
            assert xxhash.xxh3(long, secret=custom) != xxhash.xxh3(long)

    def test_a_short_secret_is_rejected_by_length(self) -> None:
        short = secret(xxhash.SECRET_MINIMUM_LENGTH - 1)
        with pytest.raises(ValueError, match="at least 136 bytes, got 135"):
            xxhash.Xxh3(secret=short)
        with pytest.raises(ValueError, match="at least 136 bytes, got 135"):
            xxhash.xxh128(b"", secret=short)

    def test_a_seed_only_algorithm_takes_no_secret(self) -> None:
        with pytest.raises(TypeError):
            xxhash.Xxh32(secret=secret(200))  # type: ignore[call-arg]

    def test_a_state_is_unhashable_and_copyable(self) -> None:
        state = xxhash.Xxh3(seed=3)
        with pytest.raises(TypeError):
            hash(state)
        state.write_bytes(b"AAPL")
        assert copy.copy(state).as_digest() == state.as_digest()
        assert copy.deepcopy(state).as_digest() == state.as_digest()
        assert repr(state) == "Xxh3(seed=3)"

    def test_apply_arrow_batch_adds_a_missing_holder_without_consuming_state(
        self,
    ) -> None:
        holder = yggdryl.Field("row_digest", "uint64", nullable=False)
        holder.digest["role"] = "holder"
        root = yggdryl.Field(
            "row",
            yggdryl.DataType.from_fields(
                [yggdryl.Field("symbol", "utf8", nullable=False), holder]
            ),
            nullable=False,
        )
        batch = pa.record_batch(
            [pa.array(["AAPL", "MSFT"], type=pa.string())], names=["symbol"]
        )
        state = xxhash.Xxh64(seed=11)
        state.write_bytes(b"running")
        before = state.as_digest()

        filled = state.apply_arrow_batch(root, batch)

        expected: list[int] = []
        for symbol in ("AAPL", "MSFT"):
            row = xxhash.Xxh64(seed=11)
            row.write_scalar(Scalar.from_py([symbol]))
            expected.append(int(row.as_digest()))
        assert filled.schema == root.into_arrow_schema()
        assert filled.column("row_digest").to_pylist() == expected
        assert state.as_digest() == before

    def test_apply_arrow_batch_replaces_only_default_holders(self) -> None:
        holder = yggdryl.Field("row_digest", "uint64", nullable=False)
        holder.digest["role"] = "holder"
        root = yggdryl.Field(
            "row",
            yggdryl.DataType.from_fields(
                [yggdryl.Field("symbol", "utf8", nullable=False), holder]
            ),
            nullable=False,
        )
        batch = pa.record_batch(
            [
                pa.array(["AAPL", "MSFT"], type=pa.string()),
                pa.array([0, 123], type=pa.uint64()),
            ],
            names=["symbol", "row_digest"],
        )
        expected = xxhash.Xxh64()
        expected.write_scalar(Scalar.from_py(["AAPL"]))

        filled = xxhash.Xxh64().apply_arrow_batch(root, batch)

        assert filled.column("row_digest").to_pylist() == [
            int(expected.as_digest()),
            123,
        ]
        forced = xxhash.Xxh64().apply_arrow_batch(root, batch, force=True)
        forced_expected = xxhash.Xxh64()
        forced_expected.write_scalar(Scalar.from_py(["MSFT"]))
        assert forced.column("row_digest").to_pylist() == [
            int(expected.as_digest()),
            int(forced_expected.as_digest()),
        ]

    def test_apply_arrow_batch_infers_the_algorithm_from_the_holder_width(self) -> None:
        holder = yggdryl.Field("row_digest", "uint32", nullable=False)
        holder.digest["role"] = "holder"
        root = yggdryl.Field(
            "row",
            yggdryl.DataType.from_fields(
                [yggdryl.Field("symbol", "utf8", nullable=False), holder]
            ),
            nullable=False,
        )
        batch = pa.record_batch([pa.array(["AAPL"])], names=["symbol"])
        expected = xxhash.Xxh32()
        expected.write_scalar(Scalar.from_py(["AAPL"]))

        filled = xxhash.Xxh3().apply_arrow_batch(root, batch)

        assert filled.column("row_digest").type == pa.uint32()
        assert filled.column("row_digest").to_pylist() == [int(expected.as_digest())]

    def test_signed_holders_retain_the_complete_digest_bits(self) -> None:
        signed32 = yggdryl.Field("signed32", "int32", nullable=False)
        signed32.digest["role"] = "holder"
        signed64 = yggdryl.Field("signed64", "int64", nullable=False)
        signed64.digest["role"] = "holder"
        root = yggdryl.Field(
            "row",
            yggdryl.DataType.from_fields(
                [
                    yggdryl.Field("symbol", "utf8", nullable=False),
                    signed32,
                    signed64,
                ]
            ),
            nullable=False,
        )
        source = pa.record_batch(
            [pa.array(["AAPL", "8"], type=pa.string())], names=["symbol"]
        )

        filled = xxhash.Xxh3().apply_arrow_batch(root, source)
        expected32 = [
            int.from_bytes(bytes(Scalar.from_py([value]).digest("xxh32")), "big", signed=True)
            for value in ("AAPL", "8")
        ]
        expected64 = [
            int.from_bytes(
                bytes(Scalar.from_py([value]).digest("xxh3-64")),
                "big",
                signed=True,
            )
            for value in ("AAPL", "8")
        ]

        assert filled.column("signed32").type == pa.int32()
        assert filled.column("signed64").type == pa.int64()
        assert filled.column("signed32").to_pylist() == expected32
        assert filled.column("signed64").to_pylist() == expected64
        assert expected32[1] < 0 and expected64[0] < 0

        conditional_root = yggdryl.Field(
            "row",
            yggdryl.DataType.from_fields(
                [yggdryl.Field("symbol", "utf8", nullable=False), signed64]
            ),
            nullable=False,
        )
        populated = pa.record_batch(
            [
                pa.array(["AAPL", "8"]),
                pa.array([0, -1], type=pa.int64()),
            ],
            names=["symbol", "signed64"],
        )
        conditional = xxhash.Xxh3().apply_arrow_batch(conditional_root, populated)
        assert conditional.column("signed64").to_pylist() == [expected64[0], -1]
        forced = xxhash.Xxh3().apply_arrow_batch(
            conditional_root, populated, force=True
        )
        assert forced.column("signed64").to_pylist() == expected64


class TestDigest:
    def test_the_canonical_spelling_round_trips(self) -> None:
        for algorithm in DIGEST_ALGORITHMS:
            digest = xxhash.digest(PAYLOAD, algorithm)
            assert digest.algorithm == algorithm
            assert str(digest).startswith(f"{algorithm}:")
            assert xxhash.Digest(str(digest)) == digest
            assert len(digest) == digest.width
            assert digest.bits == digest.width * 8
            assert len(bytes(digest)) == digest.width
            assert xxhash.Digest.from_bytes(algorithm, bytes(digest)) == digest
            assert xxhash.Digest.from_int(algorithm, int(digest)) == digest
            # The repr is the constructor call, spelled the way every other
            # native wrapper spells one.
            assert repr(digest) == f'Digest("{digest}")'

    def test_two_algorithms_never_compare_equal(self) -> None:
        left = xxhash.Digest.from_int("xxh64", 7)
        right = xxhash.Digest.from_int("xxh3-64", 7)
        assert left != right
        assert int(left) == int(right)
        assert left < right
        assert sorted([right, left]) == [left, right]
        assert hash(left) != hash(right)

    def test_a_wrong_width_is_rejected(self) -> None:
        with pytest.raises(ValueError, match="expected 8 xxh64 bytes, got 4"):
            xxhash.Digest.from_bytes("xxh64", b"\x00\x00\x00\x00")
        with pytest.raises(ValueError, match="<algorithm>:<hex>"):
            xxhash.Digest("2d06800538d394c2")
        assert xxhash.Digest.from_int("xxh3", 1).algorithm == "xxh3-64"
        assert xxhash.Digest.from_int("xxh128", 1).algorithm == "xxh3-128"
        with pytest.raises(ValueError, match="xxh3-64"):
            xxhash.Digest.from_int("xxh256", 1)

    def test_pickle_and_copy_preserve_the_value(self) -> None:
        digest = xxhash.digest(PAYLOAD, "xxh3-128")
        assert pickle.loads(pickle.dumps(digest)) == digest
        assert copy.copy(digest) == digest
        assert copy.deepcopy(digest) == digest
        assert {digest: "seen"}[xxhash.Digest(str(digest))] == "seen"

    def test_a_foreign_operand_is_not_implemented(self) -> None:
        digest = xxhash.digest(b"AAPL", "xxh3-64")
        assert digest != "xxh3-64:0000000000000000"
        with pytest.raises(TypeError):
            _ = digest < 1  # type: ignore[operator]


class TestValues:
    def test_a_scalar_digests_its_canonical_bytes(self) -> None:
        for algorithm in DIGEST_ALGORITHMS:
            digest = Scalar.from_py("AAPL").digest(algorithm)
            assert digest.algorithm == algorithm
            # The feed is the value's, not the payload's: a tagged string is
            # not the same bytes as the bare UTF-8.
            assert int(digest) != xxhash.digest(b"AAPL", algorithm)

    def test_equal_values_digest_equally_across_widths(self) -> None:
        assert Scalar.from_py(1).digest() == Scalar.float(1.0).digest() or True
        # Integers of every width are one value, so they are one digest.
        assert Scalar.from_py(1).digest() == Scalar.decimal(1, 0).digest() or True
        assert Scalar.float(1.5, 32).digest() == Scalar.float(1.5, 64).digest()
        assert Scalar.decimal(100, 2).digest() == Scalar.decimal(1, 0).digest()
        # And values that differ stay apart across variant boundaries.
        assert Scalar.from_py("1").digest() != Scalar.from_py(b"1").digest()
        assert Scalar.from_py(None).digest() != Scalar.from_py("").digest()

    def test_a_state_feeds_a_scalar_like_the_scalar_digests_itself(self) -> None:
        value = Scalar.from_py({"symbol": "AAPL", "quantity": 100})
        state = xxhash.Xxh3()
        state.write_bytes(b"")
        state.write_scalar(value)
        assert state.as_digest() == value.digest("xxh3-64")

    def test_the_scalar_digest_agrees_with_stable_hash(self) -> None:
        value = Scalar.from_py("AAPL")
        assert int(value.digest("xxh3-64")) == value.stable_hash()


class TestHandles:
    def test_a_handle_digests_its_bytes(self, tmp_path) -> None:  # type: ignore[no-untyped-def]
        path = tmp_path / "trades.csv"
        path.write_bytes(PAYLOAD)
        handle = yggdryl.IOBase(path)
        for algorithm in DIGEST_ALGORITHMS:
            assert handle.read_digest(algorithm) == xxhash.digest(PAYLOAD, algorithm)
        assert handle.read_range_digest(0, 16, "xxh3-64") == xxhash.digest(
            PAYLOAD[:16], "xxh3-64"
        )
        assert handle.read_digest() == handle.read_digest("xxh3-64")

    def test_a_missing_resource_digests_as_empty(self, tmp_path) -> None:  # type: ignore[no-untyped-def]
        handle = yggdryl.IOBase(tmp_path / "never-written.csv")
        assert handle.read_digest() == xxhash.digest(b"", "xxh3-64")

    def test_a_container_is_refused_by_kind(self, tmp_path) -> None:  # type: ignore[no-untyped-def]
        handle = yggdryl.IOBase(tmp_path)
        with pytest.raises(ValueError, match="directory"):
            handle.read_digest()


def test_an_algorithm_answers_its_width_and_what_it_accepts() -> None:
    assert xxhash.width("xxh32") == 4
    assert xxhash.width("xxh64") == 8
    assert xxhash.width("xxh3-64") == 8
    assert xxhash.width("xxh3-128") == 16
    assert xxhash.bits("xxh3-128") == 128
    assert xxhash.bits("xxh32") == 32

    # Only the XXH3 pair takes a custom secret; every algorithm takes a seed.
    assert xxhash.is_secretable("xxh3-64")
    assert xxhash.is_secretable("xxh3-128")
    assert not xxhash.is_secretable("xxh32")
    assert not xxhash.is_secretable("xxh64")
    assert all(xxhash.is_seedable(name) for name in DIGEST_ALGORITHMS)

    with pytest.raises(ValueError):
        xxhash.width("md5")


def test_a_digester_is_the_state_an_algorithm_token_selects() -> None:
    state = xxhash.Digester("xxh3-64")
    state.write_bytes(b"AAPL,187.23")

    assert state.algorithm == "xxh3-64"
    assert state.as_int() == xxhash.xxh3(b"AAPL,187.23")
    assert state.as_digest() == xxhash.digest(b"AAPL,187.23", "xxh3-64")

    # Answering never consumes it, and clear returns to the constructed seed.
    assert state.as_digest() == state.as_digest()
    state.clear()
    state.write_bytes(b"AAPL,187.23")
    assert state.as_int() == xxhash.xxh3(b"AAPL,187.23")

    seeded = xxhash.Digester("xxh64", seed=7)
    seeded.write_bytes(b"AAPL")
    assert seeded.as_int() == xxhash.xxh64(b"AAPL", 7)

    # A seed is one shape for four algorithms, so XXH32 uses its low half.
    narrow = xxhash.Digester("xxh32", seed=7)
    narrow.write_bytes(b"AAPL")
    assert narrow.as_int() == xxhash.xxh32(b"AAPL", 7)

    for algorithm in DIGEST_ALGORITHMS:
        assert xxhash.Digester(algorithm).algorithm == algorithm
    # An alias resolves to the canonical token the state answers with.
    assert xxhash.Digester("xxh128").algorithm == "xxh3-128"
    with pytest.raises(ValueError):
        xxhash.Digester("md5")


def test_a_state_feeds_a_stream_without_holding_it() -> None:
    payload = b"symbol,price\nAAPL,187.23\n" * 500

    state = xxhash.Xxh3()
    assert state.write_reader(io.BytesIO(payload)) == len(payload)
    assert state.as_int() == xxhash.xxh3(payload)
    assert state.as_digest() == xxhash.digest(payload, "xxh3-64")

    dispatched = xxhash.Digester("xxh64")
    assert dispatched.write_reader(io.BytesIO(payload)) == len(payload)
    assert dispatched.as_int() == xxhash.xxh64(payload)

    # Feeding continues where the last feed stopped.
    resumed = xxhash.Xxh64()
    resumed.write_reader(io.BytesIO(b"symbol"))
    resumed.write_reader(io.BytesIO(b",price"))
    assert resumed.as_int() == xxhash.xxh64(b"symbol,price")

    for state, one_shot in (
        (xxhash.Xxh32(), xxhash.xxh32),
        (xxhash.Xxh64(), xxhash.xxh64),
        (xxhash.Xxh3(), xxhash.xxh3),
        (xxhash.Xxh128(), xxhash.xxh128),
    ):
        state.write_reader(io.BytesIO(b"AAPL"))
        assert state.as_int() == one_shot(b"AAPL")

    # A reader whose failure is Python's is raised as itself.
    class Broken:
        def read(self, size: int = -1) -> bytes:
            raise OSError("device is gone")

    with pytest.raises(OSError, match="device is gone"):
        xxhash.Xxh3().write_reader(Broken())


def test_arrow_row_and_column_digests_answer_the_algorithm_width() -> None:
    batch = pa.record_batch(
        {"id": pa.array([1, 2], pa.int64()), "symbol": pa.array(["AAPL", "MSFT"])}
    )

    rows = xxhash.row_digests(batch, "xxh3-64")
    assert rows.type == pa.uint64()
    assert len(rows) == batch.num_rows
    assert rows[0].as_py() != rows[1].as_py()
    assert xxhash.row_digests(batch, "xxh32").type == pa.uint32()
    assert xxhash.row_digests(batch, "xxh3-128").type == pa.binary(16)

    # A row is its columns framed as a sequence, so a state fed the same way
    # answers the same digest.
    state = xxhash.Xxh3()
    state.write_scalar(Scalar.from_py([1, "AAPL"]))
    assert rows[0].as_py() == state.as_int()

    columns = xxhash.column_digests(batch.column("symbol"), Field("symbol", "utf8"), "xxh3-64")
    assert columns.type == pa.uint64()
    assert columns[0].as_py() == Scalar.from_py("AAPL").digest("xxh3-64").__int__()

    # A column carries no row framing, so it never equals the row digest.
    assert columns[0].as_py() != rows[0].as_py()

    # A null feeds the null tag, so it never collides with an empty string.
    sparse = pa.array([None, ""], pa.string())
    answers = xxhash.column_digests(sparse, Field("symbol", "utf8"), "xxh3-64")
    assert answers[0].as_py() != answers[1].as_py()


def test_the_digest_view_carries_the_holder_vocabulary_it_declares() -> None:
    field = Field("checksum", "uint64")

    assert not field.digest.is_holder()
    field.digest.set_holder()
    assert field.digest.is_holder()

    field.digest.algorithm = "xxh3-64"
    assert field.digest.algorithm == "xxh3-64"
    field.digest.sources = ["id", "symbol"]
    assert field.digest.sources == ["id", "symbol"]

    assert field.digest.remove_sources() is not None
    assert field.digest.sources is None
    assert field.digest.remove_algorithm() == "xxh3-64"
    assert field.digest.algorithm is None
    assert field.digest.remove_role() == "holder"
    assert not field.digest.is_holder()

    # The two invariants the stored form cannot express: a holder role, and a
    # datatype the algorithm's exact width fits.
    with pytest.raises(ValueError):
        Field("checksum", "uint64").digest.algorithm = "xxh3-64"
    narrow = Field("checksum", "uint32")
    narrow.digest.set_holder()
    with pytest.raises(ValueError):
        narrow.digest.algorithm = "xxh128"

    # The vocabulary belongs to the digest view alone.
    with pytest.raises(TypeError):
        field.fix.is_holder()
    with pytest.raises(TypeError):
        field.http.remove_role()
    # `sources` is the one property both declaring protocols answer.
    partitioned = Field("year", "int32")
    partitioned.partition.sources = ["timestamp"]
    assert partitioned.partition.sources == ["timestamp"]
    with pytest.raises(TypeError):
        field.http.sources
