"""An instant coupled with a digest, checked against the digest surface.

Every coupled answer is the plain ``xxhash`` answer with the instant laid in
front of it, so each case is pinned against the digest the same bytes give
elsewhere and against the C ``libxxhash`` binding where it can be.
"""

from __future__ import annotations

import copy
import datetime as dt
import pickle
import uuid

import pyarrow as pa
import pytest

import yggdryl
from yggdryl import DataType, Field, Scalar
from yggdryl.enums import DIGEST_ALGORITHMS
from yggdryl.hashing import txhash, xxhash

INSTANT = 1_700_000_000_000_000
PAYLOAD = b'{"symbol": "AAPL", "price": 187.23}\n' * 64
UTC = dt.timezone.utc
I64_MIN = -(2**63)
I64_MAX = 2**63 - 1


class TestValues:
    def test_one_shots_couple_the_instant_with_the_plain_digest(self) -> None:
        for name, coupled, plain in [
            ("xxh32", txhash.txh32, xxhash.xxh32),
            ("xxh64", txhash.txh64, xxhash.xxh64),
            ("xxh3-64", txhash.txh3, xxhash.xxh3),
            ("xxh3-128", txhash.txh128, xxhash.xxh128),
        ]:
            value = coupled(PAYLOAD, INSTANT)
            assert value.unix == INSTANT, name
            assert value.unit == "us", name
            assert value.algorithm == name
            assert int(value.digest) == plain(PAYLOAD), name
            assert value.width == txhash.width(name) == len(value) == len(bytes(value))
            assert value.dtype == txhash.dtype(name) == DataType(f"fixed_size_binary[{value.width}]")
            assert value == txhash.digest(PAYLOAD, INSTANT, name)
            seeded = coupled(PAYLOAD, INSTANT, seed=7)
            assert int(seeded.digest) == plain(PAYLOAD, seed=7), name
        assert txhash.width("xxh32") == 12
        assert txhash.width("xxh3-64") == txhash.width("xxh64") == 16
        assert txhash.width("xxh3-128") == 24
        assert txhash.DEFAULT_UNIT == "us"
        assert txhash.UNIX_WIDTH == 8
        assert DIGEST_ALGORITHMS == ("xxh32", "xxh64", "xxh3-64", "xxh3-128")

    def test_bytes_are_the_instant_then_the_digest(self) -> None:
        value = txhash.txh3(PAYLOAD, INSTANT)
        raw = bytes(value)
        assert raw[:8] == INSTANT.to_bytes(8, "big", signed=True)
        assert raw[8:] == bytes(value.digest)
        assert txhash.TxHash.from_bytes("us", "xxh3-64", raw) == value
        negative = txhash.txh3(PAYLOAD, -1)
        assert bytes(negative)[:8] == b"\xff" * 8
        with pytest.raises(ValueError):
            txhash.TxHash.from_bytes("us", "xxh3-128", raw)
        with pytest.raises(ValueError):
            txhash.TxHash.from_bytes("d", "xxh3-64", raw)

    def test_the_spelling_round_trips(self) -> None:
        value = txhash.txh64(PAYLOAD, INSTANT)
        assert str(value) == f"{INSTANT}@us:{value.digest}"
        assert txhash.TxHash(str(value)) == value
        assert repr(value) == f'TxHash("{value}")'
        with pytest.raises(ValueError):
            txhash.TxHash("1@d:xxh64:0000000000000000")
        with pytest.raises(ValueError):
            txhash.TxHash("not a coupled value")

    def test_values_order_by_instant_then_digest_and_hash_stably(self) -> None:
        earlier = txhash.txh3(b"AAPL", INSTANT)
        later = txhash.txh3(b"AAPL", INSTANT + 1)
        assert earlier < later <= later
        assert bytes(earlier) < bytes(later)
        assert earlier != later and earlier == txhash.txh3(b"AAPL", INSTANT)
        assert hash(earlier) == hash(txhash.txh3(b"AAPL", INSTANT))
        assert earlier.stable_hash() == txhash.txh3(b"AAPL", INSTANT).stable_hash()
        assert earlier != txhash.txh64(b"AAPL", INSTANT), "two algorithms are never equal"
        assert earlier.__eq__(1) is NotImplemented
        assert len({earlier, later, txhash.txh3(b"AAPL", INSTANT)}) == 2

    def test_pickle_and_copy_preserve_the_value(self) -> None:
        value = txhash.txh128(PAYLOAD, INSTANT)
        assert pickle.loads(pickle.dumps(value)) == value
        assert copy.copy(value) == value
        assert copy.deepcopy(value) == value

    def test_from_parts_reads_every_instant_spelling(self) -> None:
        digest = xxhash.digest(b"AAPL", "xxh3-64")
        integer = txhash.TxHash.from_parts(INSTANT, digest)
        assert integer == txhash.txh3(b"AAPL", INSTANT)
        aware = dt.datetime(2023, 11, 14, 22, 13, 20, tzinfo=UTC)
        assert txhash.TxHash.from_parts(aware, digest) == integer
        kolkata = aware.astimezone(dt.timezone(dt.timedelta(hours=5, minutes=30)))
        assert txhash.TxHash.from_parts(kolkata, digest) == integer, "a zone moves nothing"
        naive = dt.datetime(2023, 11, 14, 22, 13, 20)
        assert txhash.TxHash.from_parts(naive, digest) == integer, "naive reads as UTC"
        assert txhash.TxHash.from_parts("2023-11-14T22:13:20Z", digest) == integer
        assert txhash.TxHash.from_parts(Scalar.from_(aware), digest) == integer
        day = txhash.TxHash.from_parts(dt.date(1970, 1, 2), digest)
        assert day.unix == 86_400_000_000
        seconds = txhash.TxHash.from_parts(aware, digest, unit="s")
        assert seconds.unix == 1_700_000_000 and seconds.unit == "s"
        assert seconds == integer.with_unit("s")
        assert seconds != integer, "two units are never equal"
        with pytest.raises(TypeError):
            txhash.TxHash.from_parts(True, digest)
        with pytest.raises(ValueError):
            txhash.TxHash.from_parts(None, digest)
        with pytest.raises(ValueError):
            txhash.TxHash.from_parts(dt.time(1, 2), digest)
        with pytest.raises(ValueError):
            txhash.TxHash.from_parts(INSTANT, digest, unit="d")

    def test_projections_answer_the_instant_and_the_cell(self) -> None:
        value = txhash.txh3(b"AAPL", INSTANT)
        instant = value.into_datetime()
        assert instant.as_py() == dt.datetime(2023, 11, 14, 22, 13, 20, tzinfo=UTC)
        assert instant == DataType('datetime64(us,"UTC")').scalar(INSTANT)
        cell = value.into_scalar()
        assert cell.as_bytes() == bytes(value)
        assert cell.dtype == DataType("fixed_size_binary[16]")
        assert value.with_unit("ns").unix == INSTANT * 1_000
        assert value.with_unit("s").into_datetime().as_py() == instant.as_py()
        with pytest.raises(ValueError):
            value.with_unit("day_time")

    def test_into_uuid_packs_the_microsecond_instant_as_a_uuidv7(self) -> None:
        # Pinned by rust/tests/hashing/txhash.rs and the `TxHash::into_uuid` doctest.
        one = txhash.TxHash.from_parts(0, xxhash.Digest.from_int("xxh64", 1), unit="ns")
        assert one.into_uuid().as_py() == "00000000-0000-7000-8000-000000000001"
        digest = xxhash.Digest.from_int("xxh64", 0x0123_4567_89AB_CDEF)
        for nanoseconds, expected in [
            (0, "00000000-0000-7000-8123-456789abcdef"),
            # The sub-microsecond nanoseconds are floored away.
            (999, "00000000-0000-7000-8123-456789abcdef"),
            (1_000, "00000000-0000-7004-8123-456789abcdef"),
            (999_999, "00000000-0000-7ffb-8123-456789abcdef"),
            (1_000_000, "00000000-0001-7000-8123-456789abcdef"),
            (1_000_000_000, "00000000-03e8-7000-8123-456789abcdef"),
            (I64_MAX, "08637bd0-5af6-7c66-8123-456789abcdef"),
        ]:
            value = txhash.TxHash.from_parts(nanoseconds, digest, unit="ns")
            raw = bytes(value)
            projected = value.into_uuid()
            assert projected.as_py() == expected, nanoseconds
            assert projected.dtype == DataType("uuid")
            outside = uuid.UUID(projected.as_py())
            assert outside.version == 7 and outside.variant == uuid.RFC_4122
            assert bytes(value) == raw, "projection does not mutate the value"
            assert raw[:8] == nanoseconds.to_bytes(8, "big", signed=True)
            assert raw[8:] == bytes(digest)

    def test_into_uuid_orders_microsecond_instants_before_every_digest_bit(self) -> None:
        instants = [0, 1_000, 15_000, 16_000, 65_535_000, 65_536_000, 1_000_000_000, I64_MAX]
        high = xxhash.Digest.from_int("xxh64", 2**128 - 1)
        low = xxhash.Digest.from_int("xxh64", 0)
        for before, after in zip(instants, instants[1:]):
            earlier = txhash.TxHash.from_parts(before, high, unit="ns")
            later = txhash.TxHash.from_parts(after, low, unit="ns")
            assert earlier < later, "native ordering still compares the signed count"
            assert earlier.into_uuid() < later.into_uuid(), (before, after)
        # Within one microsecond the instant ties, and the digest orders.
        low_digest = txhash.TxHash.from_parts(1_000, xxhash.Digest.from_int("xxh64", 1), unit="ns")
        high_digest = txhash.TxHash.from_parts(1_999, xxhash.Digest.from_int("xxh64", 2), unit="ns")
        assert low_digest.into_uuid() < high_digest.into_uuid()
        # Before the epoch there is no UUIDv7 to project, while the raw bytes
        # still hold the two's-complement count.
        negative = txhash.TxHash.from_parts(-1, low, unit="ns")
        epoch = txhash.TxHash.from_parts(0, low, unit="ns")
        with pytest.raises(ValueError, match="UUIDv7"):
            negative.into_uuid()
        assert bytes(negative) > bytes(epoch), "raw bytes retain two's-complement ordering"

    def test_into_uuid_normalizes_units_and_preserves_restatement_overflow(self) -> None:
        digest = xxhash.Digest.from_int("xxh3-64", 7)
        for seconds in [0, 2]:
            expected = txhash.TxHash.from_parts(seconds, digest, unit="s").into_uuid()
            for unit, scale in [("s", 1), ("ms", 1_000), ("us", 1_000_000), ("ns", 1_000_000_000)]:
                value = txhash.TxHash.from_parts(seconds * scale, digest, unit=unit)
                assert value.into_uuid() == expected, unit
        for unit, scale in [("s", 1_000_000_000), ("ms", 1_000_000), ("us", 1_000)]:
            # Rust's `i64::MIN / scale` truncates toward zero.
            lowest, highest = -(2**63 // scale), I64_MAX // scale
            txhash.TxHash.from_parts(highest, digest, unit=unit).into_uuid()
            # A count below the epoch restates, and is then refused as a UUIDv7.
            with pytest.raises(ValueError, match="UUIDv7"):
                txhash.TxHash.from_parts(lowest, digest, unit=unit).into_uuid()
            for count in [lowest - 1, highest + 1]:
                with pytest.raises(ValueError) as restated:
                    txhash.restate_unix(count, unit, "ns")
                with pytest.raises(ValueError) as projected:
                    txhash.TxHash.from_parts(count, digest, unit=unit).into_uuid()
                assert str(projected.value) == str(restated.value)
                assert "unix restatement" in str(projected.value)

    def test_into_uuid_discards_only_the_high_two_digest_bits_and_algorithm(self) -> None:
        def project(algorithm: str, payload: int) -> Scalar:
            digest = xxhash.Digest.from_int(algorithm, payload)
            return txhash.TxHash.from_parts(1, digest, unit="ns").into_uuid()

        payload = 0x0123_4567_89AB_CDEF
        expected = project("xxh64", payload)
        assert project("xxh3-64", payload) == expected
        for bit in range(62, 64):
            assert project("xxh64", payload ^ (1 << bit)) == expected, bit
        for bit in range(62):
            assert project("xxh64", payload ^ (1 << bit)) != expected, bit

    def test_into_uuid_refuses_non_64_bit_digests_without_narrowing(self) -> None:
        for algorithm, bits in [("xxh32", 32), ("xxh3-128", 128)]:
            for payload in [0, 7, 2**128 - 1]:
                digest = xxhash.Digest.from_int(algorithm, payload)
                value = txhash.TxHash.from_parts(0, digest, unit="ns")
                with pytest.raises(ValueError) as refused:
                    value.into_uuid()
                message = str(refused.value)
                assert "$.digest" in message
                assert message.endswith(
                    f"expected a 64-bit digest for UUIDv7, got {algorithm} ({bits} bits)"
                )

    def test_instant_helpers_read_the_same_way_everywhere(self) -> None:
        aware = dt.datetime(2023, 11, 14, 22, 13, 20, tzinfo=UTC)
        assert txhash.unix_of(aware) == INSTANT
        assert txhash.unix_of(aware, unit="s") == 1_700_000_000
        assert txhash.unix_of(INSTANT) == INSTANT
        assert txhash.unix_of("1970-01-01T00:00:01+01:00") == -3_599_000_000
        assert txhash.restate_unix(1_999, "ns", "us") == 1
        assert txhash.restate_unix(-1, "ns", "us") == -1
        assert txhash.restate_unix(1, "d", "s") == 86_400
        with pytest.raises(ValueError):
            txhash.restate_unix(1, "s", "d")
        now = txhash.unix_now()
        assert now // 1_000_000 >= txhash.unix_now("s") - 1
        assert now > INSTANT
        with pytest.raises(ValueError):
            txhash.unix_now("d")


class TestHasher:
    def test_a_hasher_carries_unit_seed_and_algorithm(self) -> None:
        plain = txhash.TxHasher()
        assert plain.unit == "us" and plain.algorithm == "xxh3-64" and plain.width == 16
        assert plain.dtype == DataType("fixed_size_binary[16]")
        assert plain.digest(PAYLOAD, INSTANT) == txhash.txh3(PAYLOAD, INSTANT)
        assert repr(plain) == 'TxHasher("xxh3-64", unit="us")'

        seconds = txhash.TxHasher("xxh64", unit="s", seed=7)
        aware = dt.datetime(2023, 11, 14, 22, 13, 20, tzinfo=UTC)
        value = seconds.digest(PAYLOAD, aware)
        assert value.unit == "s" and value.unix == 1_700_000_000
        assert int(value.digest) == xxhash.xxh64(PAYLOAD, seed=7)
        assert seconds.unix_of(aware) == 1_700_000_000
        row = seconds.digest_scalar(Scalar.from_(["AAPL", 100]), INSTANT // 1_000_000)
        state = xxhash.Xxh64(seed=7)
        state.write_scalar(Scalar.from_(["AAPL", 100]))
        assert row.digest == state.as_digest()
        with pytest.raises(ValueError):
            txhash.TxHasher(unit="d")
        with pytest.raises(ValueError):
            txhash.TxHasher("md5")
        assert copy.copy(seconds).unit == "s"
        with pytest.raises(TypeError):
            hash(seconds)

    def test_a_secret_travels_through_a_configured_state(self) -> None:
        secret = bytes(range(xxhash.SECRET_MINIMUM_LENGTH))
        state = xxhash.Xxh3(seed=3, secret=secret)
        state.write_bytes(b"already fed and forgotten")
        hasher = txhash.TxHasher.from_state(state, unit="ms")
        long = b"\x11" * 241
        assert int(hasher.digest(long, 5).digest) == xxhash.xxh3(long, seed=3, secret=secret)
        assert hasher.unit == "ms" and hasher.algorithm == "xxh3-64"
        narrow = txhash.TxHasher.from_state(xxhash.Xxh32(seed=9))
        assert int(narrow.digest(PAYLOAD, 1).digest) == xxhash.xxh32(PAYLOAD, seed=9)
        dispatched = txhash.TxHasher.from_state(xxhash.Digester("xxh3-128", seed=2))
        assert dispatched.width == 24
        with pytest.raises(TypeError):
            txhash.TxHasher.from_state("xxh3-64")


def event_batch() -> pa.RecordBatch:
    return pa.record_batch(
        {
            "event": pa.array(
                [INSTANT, INSTANT + 1, INSTANT], pa.timestamp("us", tz="UTC")
            ),
            "symbol": pa.array(["AAPL", "MSFT", "AAPL"]),
        }
    )


class TestColumns:
    def test_row_txhashes_couple_row_digests_with_instants(self) -> None:
        batch = event_batch()
        for algorithm in DIGEST_ALGORITHMS:
            coupled = txhash.row_txhashes(batch, batch.column("event"), algorithm=algorithm)
            assert coupled.type == pa.binary(txhash.width(algorithm)), algorithm
            times, digests = txhash.decompose(coupled, algorithm=algorithm)
            assert digests == xxhash.row_digests(batch, algorithm), algorithm
            assert times == batch.column("event"), algorithm
            first = txhash.TxHash.from_bytes("us", algorithm, coupled[0].as_py())
            expected = Scalar.from_([dt.datetime(2023, 11, 14, 22, 13, 20, tzinfo=UTC), "AAPL"])
            assert first == txhash.TxHash.from_parts(INSTANT, expected.digest(algorithm))
            assert coupled[0] == coupled[2] and coupled[0] != coupled[1]
            assert txhash.compose(times, digests, algorithm=algorithm) == coupled

    def test_a_null_instant_is_a_null_cell_and_columns_reconcile(self) -> None:
        batch = event_batch()
        times = pa.array([1, None, 3], pa.timestamp("s"))
        coupled = txhash.row_txhashes(batch, times, unit="s")
        assert coupled.null_count == 1 and coupled[1].as_py() is None
        assert txhash.TxHash.from_bytes("s", "xxh3-64", coupled[0].as_py()).unix == 1
        column = txhash.column_txhashes(times, batch.column("symbol"), Field("symbol", "utf8"), unit="s")
        assert column[1].as_py() is None
        cell = txhash.TxHash.from_bytes("s", "xxh3-64", column[0].as_py())
        assert cell.digest == Scalar.from_("AAPL").digest()
        narrow = txhash.column_txhashes(times, pa.array([1, 2, 3], pa.int32()), Field("q", "int64"), unit="s")
        wide = txhash.column_txhashes(times, pa.array([1, 2, 3], pa.int64()), Field("q", "int64"), unit="s")
        assert narrow == wide
        with pytest.raises(ValueError):
            txhash.row_txhashes(batch, pa.array([1], pa.int64()))
        with pytest.raises(ValueError):
            txhash.row_txhashes(batch, pa.array(["x", "y", "z"]))

    def test_unix_array_reads_timestamps_dates_and_integers(self) -> None:
        assert txhash.unix_array(pa.array([1, None], pa.timestamp("s"))).to_pylist() == [1_000_000, None]
        assert txhash.unix_array(pa.array([1_999, -1], pa.timestamp("ns"))).to_pylist() == [1, -1]
        assert txhash.unix_array(pa.array([dt.date(1970, 1, 2)])).to_pylist() == [86_400_000_000]
        assert txhash.unix_array(pa.array([42], pa.int64())).to_pylist() == [42]
        assert txhash.unix_array(pa.array([INSTANT], pa.timestamp("us")), "s").to_pylist() == [1_700_000_000]
        with pytest.raises(ValueError):
            txhash.unix_array(pa.array(["2024-01-01"]))

    def test_a_seeded_hasher_couples_seeded_digests_over_columns(self) -> None:
        batch = event_batch()
        hasher = txhash.TxHasher(seed=7)
        coupled = hasher.row_txhashes(batch, batch.column("event"))
        state = xxhash.Xxh3(seed=7)
        state.write_scalar(Scalar.from_([dt.datetime(2023, 11, 14, 22, 13, 20, 1, tzinfo=UTC), "MSFT"]))
        second = txhash.TxHash.from_bytes("us", "xxh3-64", coupled[1].as_py())
        assert second.digest == state.as_digest() and second.unix == INSTANT + 1
        column = txhash.TxHasher("xxh32", unit="s").column_txhashes(
            batch.column("event"), batch.column("symbol"), Field("symbol", "utf8")
        )
        cell = txhash.TxHash.from_bytes("s", "xxh32", column[0].as_py())
        assert cell.unix == 1_700_000_000
        assert cell.digest == Scalar.from_("AAPL").digest("xxh32")


class TestHolders:
    def root(self, width: int = 16, unit: str | None = None) -> Field:
        holder = Field("key", f"fixed_size_binary[{width}]", nullable=False)
        holder.digest.set_holder()
        holder.digest.time = "event"
        if unit is not None:
            holder.digest.unit = unit
        return Field(
            "row",
            DataType.from_fields(
                [
                    Field("event", "timestamp[us, UTC]", nullable=False),
                    Field("symbol", "utf8", nullable=False),
                    holder,
                ]
            ),
            nullable=False,
        )

    def test_the_digest_view_declares_the_coupling(self) -> None:
        holder = Field("key", "fixed_size_binary[16]", nullable=False)
        assert not holder.digest.is_coupled()
        holder.digest.set_holder()
        assert holder.digest.time is None and holder.digest.unit is None
        holder.digest.time = "event"
        assert holder.digest.is_coupled()
        assert holder.digest.time == "event" and holder.metadata["digest:time"] == "event"
        holder.digest.unit = "seconds"
        assert holder.digest.unit == "s" and holder.metadata["digest:unit"] == "s"
        with pytest.raises(ValueError):
            holder.digest.remove_time()
        assert holder.digest.remove_unit() == "s"
        assert holder.digest.remove_time() == "event"
        narrow = Field("key", "uint64", nullable=False)
        narrow.digest.set_holder()
        with pytest.raises(ValueError):
            narrow.digest.time = "event"
        with pytest.raises(ValueError):
            holder.digest.unit = "s"
        with pytest.raises(TypeError):
            holder.partition.time  # noqa: B018

    def test_a_coupled_holder_is_filled_with_the_instant_in_front(self) -> None:
        root = self.root()
        batch = event_batch()
        filled = root.digest.apply_arrow_batch(batch)
        assert filled.schema == root.into_arrow_schema()
        assert filled.column("key").type == pa.binary(16)
        second = txhash.TxHash.from_bytes("us", "xxh3-64", filled.column("key")[1].as_py())
        expected = Scalar.from_([dt.datetime(2023, 11, 14, 22, 13, 20, 1, tzinfo=UTC), "MSFT"])
        assert second == txhash.TxHash.from_parts(INSTANT + 1, expected.digest())
        assert root.apply_arrow_batch(batch) == filled
        assert root.apply_arrow_batch(filled) == filled, "filling again changes nothing"
        seeded = txhash.TxHasher(seed=7).apply_arrow_batch(root, batch)
        state = xxhash.Xxh3(seed=7)
        state.write_scalar(expected)
        assert txhash.TxHash.from_bytes("us", "xxh3-64", seeded.column("key")[1].as_py()).digest == state.as_digest()

    def test_a_coupled_holder_resolves_width_and_unit(self) -> None:
        batch = event_batch()
        wide = self.root(24, "s")
        filled = wide.digest.apply_arrow_batch(batch)
        value = txhash.TxHash.from_bytes("s", "xxh3-128", filled.column("key")[0].as_py())
        assert value.unix == 1_700_000_000
        assert value.digest.algorithm == "xxh3-128"
        narrow = self.root(12)
        filled = narrow.digest.apply_arrow_batch(batch)
        assert txhash.TxHash.from_bytes("us", "xxh32", filled.column("key")[0].as_py()).unix == INSTANT

    def test_a_null_instant_nulls_a_nullable_holder_and_refuses_a_required_one(self) -> None:
        batch = pa.record_batch(
            {
                "event": pa.array([INSTANT, None], pa.timestamp("us", tz="UTC")),
                "symbol": pa.array(["AAPL", "MSFT"]),
            }
        )
        def root(holder_nullable: bool) -> Field:
            # The instant column is nullable itself: a required one would be
            # repaired to its default by the cast before the holder saw it.
            holder = Field("key", "fixed_size_binary[16]", nullable=holder_nullable)
            holder.digest.set_holder()
            holder.digest.time = "event"
            return Field(
                "row",
                DataType.from_fields(
                    [Field("event", "timestamp[us, UTC]"), Field("symbol", "utf8", nullable=False), holder]
                ),
                nullable=False,
            )

        with pytest.raises(ValueError, match="row 1"):
            root(False).digest.apply_arrow_batch(batch)
        filled = root(True).digest.apply_arrow_batch(batch)
        assert filled.column("key")[1].as_py() is None
        assert filled.column("key")[0].as_py() is not None
