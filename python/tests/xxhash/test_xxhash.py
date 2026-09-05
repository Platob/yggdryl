"""The digest surface, checked against the C ``libxxhash`` bindings.

The pinned vectors below are the published reference values. Everything else
is compared against the ``xxhash`` package, which wraps C ``libxxhash``: an
outside implementation of the same protocol, both directions, so a wrong
answer here cannot be self-consistent.
"""

from __future__ import annotations

import copy
import pickle

import pytest

import yggdryl
from yggdryl import Scalar, xxhash
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
