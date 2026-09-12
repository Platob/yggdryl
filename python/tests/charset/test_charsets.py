"""The character encodings agree with the standard library's codec registry."""

from __future__ import annotations

import pytest

import yggdryl
from yggdryl import IOBase, MediaType, charset

# The charset as this package spells it, and as ``codecs`` knows it.
PYTHON_NAME = {
    "utf-8": "utf-8",
    "utf-16le": "utf-16-le",
    "utf-16be": "utf-16-be",
    "us-ascii": "ascii",
    "iso-8859-1": "iso-8859-1",
    "iso-8859-2": "iso-8859-2",
    "iso-8859-15": "iso-8859-15",
    "windows-1250": "cp1250",
    "windows-1251": "cp1251",
    "windows-1252": "cp1252",
    "ibm437": "cp437",
    "ibm850": "cp850",
    "macintosh": "mac-roman",
}


class TestCharsets:
    def test_the_listing_is_the_vocabulary_the_names_resolve_to(self) -> None:
        assert set(charset.CHARSETS) == set(PYTHON_NAME)
        assert charset.CHARSETS == yggdryl.enums.CHARSETS
        for name in charset.CHARSETS:
            assert charset.canonical_name(name) == name

    def test_customary_aliases_resolve_to_one_name(self) -> None:
        assert charset.canonical_name("cp1252") == "windows-1252"
        assert charset.canonical_name("latin1") == "iso-8859-1"
        assert charset.canonical_name("UTF8") == "utf-8"
        # Two standards disagree over bare "utf-16", so it is refused rather
        # than defaulted to either order.
        with pytest.raises(ValueError):
            charset.canonical_name("utf-16")

    def test_every_charset_agrees_with_the_codec_registry(self) -> None:
        text = "symbol,price\nAAPL,187.23\n"
        for name, codec in PYTHON_NAME.items():
            assert charset.decode(name, text.encode(codec)) == text
            assert charset.encode(name, text) == text.encode(codec)

    def test_the_western_code_pages_disagree_where_they_should(self) -> None:
        wire = b"caf\xe9 \x80"
        assert charset.decode("windows-1252", wire) == wire.decode("cp1252")
        assert charset.decode("iso-8859-1", wire) == wire.decode("iso-8859-1")
        assert charset.decode("windows-1252", wire) != charset.decode("iso-8859-1", wire)

    def test_a_refusal_names_the_charset_and_what_it_found(self) -> None:
        with pytest.raises(ValueError) as decoding:
            charset.decode("us-ascii", b"caf\xe9")
        assert "us-ascii" in str(decoding.value)
        with pytest.raises(ValueError) as encoding:
            charset.encode("iso-8859-1", "prix 12€")
        assert "U+20AC" in str(encoding.value)
        # The lossy door reads the same bytes without refusing them.
        assert charset.decode_lossy("us-ascii", b"caf\xe9") == "caf�"

    def test_a_byte_order_mark_names_its_charset_and_its_length(self) -> None:
        assert charset.from_bom(b"\xff\xfeA\x00") == ("utf-16le", 2)
        assert charset.from_bom(b"\xef\xbb\xbfid") == ("utf-8", 3)
        assert charset.from_bom(b"id,name") is None
        assert charset.bom("utf-8") == b"\xef\xbb\xbf"
        assert charset.bom("windows-1252") is None

    def test_a_media_type_carries_the_charset_it_declares(self) -> None:
        declared = MediaType.from_str("text/csv;charset=windows-1252")
        assert declared.charset == "windows-1252"
        assert str(declared) == "text/csv;charset=windows-1252"
        assert MediaType.from_str(str(declared)) == declared
        assert MediaType.from_str("text/csv").charset is None

        mutable = MediaType.from_str("text/csv")
        mutable.set_charset("latin1")
        assert mutable.charset == "iso-8859-1"
        mutable.set_charset(None)
        assert mutable.charset is None

    def test_a_declared_charset_is_read_by_the_record_reader(self) -> None:
        # The record reader reads the declaration off the handle's media type
        # and lays it over its transport, below the line splitter, so a body
        # of one windows-1252 byte per scalar arrives as `str`.
        wire = "Zürich premièr\n".encode("cp1252")
        assert wire == b"Z\xfcrich premi\xe8r\n"
        handle = IOBase.from_bytes(wire)
        handle.media_type = "text/plain;charset=windows-1252"
        [row] = handle.read_records()
        assert isinstance(row["body"], str)
        assert row["body"] == "Zürich premièr"
