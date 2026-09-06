"""A handle's name says what coding its bytes carry, and the transfers use it.

``compress_into`` and ``decompress_into`` move bytes between two handles rather
than through Python, so what is asserted here is the contract that makes them
usable without a second spelling of the coding: the target's own name says which
coding to write, and a name that says nothing is refused instead of guessed.
"""

from __future__ import annotations

import gzip as std_gzip
import pathlib

import pytest

from yggdryl import IOBase
from yggdryl.holder import Path

PAYLOAD = '{"id": 1, "venue": "XNAS"}\n' * 64


@pytest.fixture
def plain(tmp_path: pathlib.Path) -> IOBase:
    """A real file on disk, holding bytes worth compressing."""
    handle = IOBase(tmp_path / "rows.json")
    handle.write_text(PAYLOAD)
    return handle


class TestDeclaredCoding:
    """``codec`` is what the name says the bytes are wrapped in."""

    def test_a_plain_name_declares_no_coding(self, tmp_path: pathlib.Path) -> None:
        # Identity is spelled ``None`` rather than ``"identity"``, because the
        # question a caller asks is "is anything wrapped around these bytes".
        assert IOBase(tmp_path / "rows.json").codec is None
        assert IOBase(tmp_path / "rows").codec is None
        assert IOBase.from_bytes(b"{}").codec is None

    def test_a_compound_name_declares_the_coding_its_last_extension_names(
        self, tmp_path: pathlib.Path
    ) -> None:
        assert IOBase(tmp_path / "rows.json.gz").codec == "gzip"
        assert IOBase(tmp_path / "rows.json.zst").codec == "zstd"
        assert IOBase(tmp_path / "rows.json.zz").codec == "zlib"

    def test_the_coding_is_read_from_the_name_and_not_from_the_bytes(
        self, plain: IOBase, tmp_path: pathlib.Path
    ) -> None:
        misnamed = IOBase(tmp_path / "rows.json.gz")
        misnamed.write_text(PAYLOAD)

        # Nothing was read to answer, which is what lets a caller ask before
        # paying for a byte - and what makes a wrong name the caller's error.
        assert misnamed.codec == "gzip"
        assert plain.codec is None


class TestCompressInto:
    """Encoding into a target is named by the target, or refused."""

    def test_a_gz_target_round_trips_through_a_real_file_on_disk(
        self, plain: IOBase, tmp_path: pathlib.Path
    ) -> None:
        coded = Path(tmp_path / "rows.json.gz")
        back = IOBase(tmp_path / "back.json")

        written = plain.compress_into(coded)
        read = coded.decompress_into(back)

        # Both calls report bytes rather than nothing: the encoded size is what
        # landed, and the decoded size is what came out.
        assert written == coded.size
        assert written < len(PAYLOAD)
        assert read == len(PAYLOAD)
        assert back.read_text() == PAYLOAD
        assert (tmp_path / "rows.json.gz").exists()

    def test_the_file_it_wrote_is_a_gzip_the_standard_library_reads(
        self, plain: IOBase, tmp_path: pathlib.Path
    ) -> None:
        coded = Path(tmp_path / "rows.json.gz")
        plain.compress_into(coded)

        # The name promised gzip, so anything that reads gzip has to be able to
        # read it - a coding only this library can undo is not a coding.
        assert std_gzip.decompress((tmp_path / "rows.json.gz").read_bytes()) == (
            PAYLOAD.encode()
        )
        with std_gzip.open(tmp_path / "rows.json.gz", "rt") as stream:
            assert stream.read() == PAYLOAD

    def test_a_target_naming_no_coding_is_refused_rather_than_copied(
        self, plain: IOBase, tmp_path: pathlib.Path
    ) -> None:
        target = IOBase(tmp_path / "copy.json")

        # Silently copying would leave bytes nobody can decode by name later,
        # so the refusal reports what the target does declare - a media type
        # carrying no coding - and says how to say what was meant.
        with pytest.raises(ValueError, match=r"got application/json; pass codec="):
            plain.compress_into(target)

        assert not (tmp_path / "copy.json").exists()

    def test_a_named_coding_writes_a_target_whose_name_says_nothing(
        self, plain: IOBase, tmp_path: pathlib.Path
    ) -> None:
        target = IOBase(tmp_path / "rows.bin")

        written = plain.compress_into(target, "zstd")

        assert written == target.size
        # The target records the coding it just received, which is what lets
        # the matching decode take no argument at all.
        assert target.codec == "zstd"
        back = IOBase(tmp_path / "back.json")
        assert target.decompress_into(back) == len(PAYLOAD)
        assert back.read_text() == PAYLOAD

    def test_the_level_reaches_the_encoder_at_both_ends_of_the_scale(
        self, plain: IOBase, tmp_path: pathlib.Path
    ) -> None:
        stored = Path(tmp_path / "stored.json.gz")
        smallest = Path(tmp_path / "small.json.gz")

        # 0 is "wrap it and store it", so it is the one level that must come
        # out larger than the input - which is how a level that never reached
        # the encoder is caught.
        assert plain.compress_into(stored, level=0) > len(PAYLOAD)
        assert plain.compress_into(smallest, level=9) < len(PAYLOAD)
        # Whatever the level, the bytes are still gzip.
        assert std_gzip.decompress((tmp_path / "stored.json.gz").read_bytes()) == (
            PAYLOAD.encode()
        )

    def test_a_coding_nobody_defines_is_refused_naming_the_ones_that_are(
        self, plain: IOBase, tmp_path: pathlib.Path
    ) -> None:
        with pytest.raises(ValueError, match="identity, gzip, zlib, deflate, zstd"):
            plain.compress_into(IOBase(tmp_path / "rows.bin"), "lzma")


class TestDecompressInto:
    """Decoding defaults to what this handle's own name declares."""

    def test_the_source_name_supplies_the_coding(
        self, plain: IOBase, tmp_path: pathlib.Path
    ) -> None:
        coded = Path(tmp_path / "rows.json.gz")
        plain.compress_into(coded)
        back = IOBase(tmp_path / "back.json")

        reopened = Path(tmp_path / "rows.json.gz")
        assert reopened.decompress_into(back) == len(PAYLOAD)
        assert back.read_text() == PAYLOAD
        # The target loses the coding this removed, so the pair is symmetric.
        assert back.codec is None

    def test_a_coding_the_bytes_are_not_is_refused(
        self, plain: IOBase, tmp_path: pathlib.Path
    ) -> None:
        coded = Path(tmp_path / "rows.json.gz")
        plain.compress_into(coded)

        # Overriding the name is allowed, so naming the wrong coding has to
        # fail on the bytes rather than write a plausible ruin.
        with pytest.raises(ValueError):
            coded.decompress_into(IOBase(tmp_path / "wrong.json"), "zstd")

    def test_plain_bytes_are_not_a_coded_value(
        self, plain: IOBase, tmp_path: pathlib.Path
    ) -> None:
        with pytest.raises(ValueError):
            plain.decompress_into(IOBase(tmp_path / "out.json"), "gzip")


class TestCodedView:
    """``into_coded`` opens a handle as the value its coding holds."""

    def test_the_path_declares_the_coding_the_view_decodes(
        self, tmp_path: pathlib.Path
    ) -> None:
        plain = "[INFO] alpha\n[WARN] beta\n"
        path = tmp_path / "app.log.gz"
        path.write_bytes(std_gzip.compress(plain.encode()))

        # The stored-byte role mirrors what is on disk, coding and all.
        source = Path(path)
        assert source.codec == "gzip"
        assert source.read_bytes()[:2] == b"\x1f\x8b"

        # The view presents what those bytes hold. The coding is the class it
        # reports, not something left in its media type.
        decoded = Path(path).into_coded()
        assert type(decoded).__name__ == "Gzip"
        assert decoded.media_type.base == source.media_type.base
        assert decoded.read_text() == plain
        assert decoded.size == len(plain)
        assert decoded.read_range_bytes(7, 5) == b"alpha"
        # Reads stream in bounded windows rather than materializing the value.
        assert b"".join(decoded.pstream_bytes(0, 8)) == plain.encode()

    def test_the_view_is_answered_idempotent_and_overridable(
        self, tmp_path: pathlib.Path
    ) -> None:
        payload = PAYLOAD.encode()
        path = tmp_path / "rows.json"
        path.write_bytes(std_gzip.compress(payload))

        # A name declaring no coding passes its bytes through unchanged.
        passthrough = IOBase(tmp_path / "plain.json")
        passthrough.write_bytes(payload)
        assert passthrough.into_coded().read_bytes() == payload

        # This name says nothing about the bytes, so the caller names it. The
        # view is a handle of its own, and the one it was built from is spent.
        stored = IOBase(path)
        handle = stored.into_coded("gzip")
        assert type(handle).__name__ == "Gzip"
        assert handle.read_bytes() == payload
        with pytest.raises(ValueError, match="consumed"):
            stored.read_bytes()

        # A view already presenting decoded bytes has no second coding to
        # remove, so repeating the call never decodes twice.
        assert handle.into_coded("zstd", 9).read_bytes() == payload

        with pytest.raises(ValueError, match="content coding"):
            IOBase(path).into_coded("brotli")

    def test_a_write_through_the_view_publishes_the_encoded_form(
        self, tmp_path: pathlib.Path
    ) -> None:
        path = tmp_path / "notes.txt.gz"
        handle = Path(path).into_coded()

        handle.write_text("alpha\n")
        handle.close()

        assert std_gzip.decompress(path.read_bytes()) == b"alpha\n"
        assert IOBase(path).read_text() == "alpha\n"
