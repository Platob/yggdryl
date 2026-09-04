"""The static enum vocabularies, and the ASCII types declared as enums."""

from __future__ import annotations

import logging

import pytest

from yggdryl import AsciiEnum, DataType, Field, enums, scalar
from yggdryl.enums import (
    AsciiCode,
    CFI,
    CfiCode,
    Country,
    CountryCode,
    Currency,
    CurrencyCode,
    MIC,
    MicCode,
    fixed_ascii,
)


def test_every_vocabulary_is_a_nonempty_tuple_of_strings() -> None:
    for listing in (
        enums.DATA_TYPE_IDS,
        enums.DATA_TYPE_KINDS,
        enums.TIME_UNITS,
        enums.UNION_MODES,
        enums.IO_MODES,
        enums.CODECS,
        enums.IO_KINDS,
        enums.COMPATIBILITY_SCHEMES,
    ):
        assert isinstance(listing, tuple)
        assert listing
        assert all(isinstance(value, str) and value for value in listing)


def test_the_spellings_are_the_ones_the_parsers_accept() -> None:
    # Every listed id parses as a datatype or is a parameterized family the
    # grammar spells with arguments; the simple ones round-trip exactly.
    assert "int64" in enums.DATA_TYPE_IDS
    assert DataType("int64").id == "int64"
    assert "integer" in enums.DATA_TYPE_KINDS
    assert enums.UNION_MODES == ("sparse", "dense")
    assert enums.IO_MODES == ("overwrite", "append", "merge", "readonly", "random")
    assert "us" in enums.TIME_UNITS
    assert "gzip" in enums.CODECS
    assert "file" in enums.IO_KINDS
    assert "arrow" in enums.COMPATIBILITY_SCHEMES


def test_the_level_scale_names_its_points() -> None:
    assert enums.LEVELS["none"] == 0
    assert enums.LEVELS["fast"] == 1
    assert enums.LEVELS["default"] == 6
    assert enums.LEVELS["best"] == 9


class _Currency(fixed_ascii(4)):
    """A declared vocabulary: a member is the integer its value packs into."""

    USD = "USD"
    EUR = "EUR"


def test_a_member_is_the_integer_its_ascii_value_packs_into() -> None:
    assert int(_Currency.USD) == 0x55534400
    assert int(_Currency.EUR) == DataType.ascii(4).ascii_packed("EUR")
    assert _Currency.dtype() == DataType.ascii(4)

    # The packed code is the bytes an ASCII column stores, so the order of the
    # integers is the order of the text and the code is the same everywhere.
    assert int(_Currency.USD).to_bytes(4, "big") == b"USD\x00"
    assert _Currency.EUR < _Currency.USD

    assert _Currency.EUR.into_str() == "EUR"
    assert str(_Currency.EUR) == "EUR"
    assert f"{_Currency.EUR}" == "EUR"
    assert f"{_Currency.EUR:>5}" == "  EUR"
    assert repr(_Currency.EUR) == f"<_Currency.EUR: 'EUR' = {0x45555200}>"


def test_the_widths_pack_into_the_integer_they_name() -> None:
    class Country(fixed_ascii(2)):
        US = "US"

    class Currency(fixed_ascii(3)):
        USD = "USD"

    class Venue(fixed_ascii(8)):
        XNAS = "XNAS"

    class Isin(fixed_ascii(12)):
        SAMPLE = "US0378331005"

    class Wide(fixed_ascii(16)):
        SAMPLE = "US0378331005XNAS"

    # ISO 3166-1 is two bytes and ISO 4217 three, so each packs with no
    # padding at all under the width its standard names.
    assert Country.dtype() == DataType.ascii(2)
    assert int(Country.US) == 0x5553
    assert Currency.dtype() == DataType.ascii(3)
    assert int(Currency.USD) == 0x555344
    assert Venue.dtype() == DataType.ascii(8)
    assert int(Venue.XNAS) == DataType.ascii(8).ascii_packed("XNAS")

    # Twelve bytes need 96 bits and sixteen the whole 128, both of which Python
    # holds natively.
    assert int(Isin.SAMPLE) == 0x555330333738333331303035
    assert int(Isin.SAMPLE).bit_length() > 64
    assert Isin.SAMPLE.into_str() == "US0378331005"
    assert int(Wide.SAMPLE).bit_length() > 96

    # One width is one base, built once and cached, so two declarations of the
    # same width name the same class.
    assert fixed_ascii(3) is Currency.__mro__[1]
    assert fixed_ascii(3).__name__ == "FixedAscii3"

    # The base names no datatype, so it is not a vocabulary of its own.
    with pytest.raises(TypeError, match="fixed_ascii"):
        AsciiCode.dtype()

    # A member *is* the packed integer, so a width with none names no
    # vocabulary, and neither does the variable form.
    with pytest.raises(ValueError, match="at most 16 bytes"):
        fixed_ascii(17)
    with pytest.raises(ValueError, match="at least 1 byte"):
        fixed_ascii(0)


def test_an_undeclared_value_registers_once_and_says_so_once(
    caplog: pytest.LogCaptureFixture,
) -> None:
    class Side(fixed_ascii(4)):
        BUY = "B"
        SELL = "S"

    with caplog.at_level(logging.INFO, logger="yggdryl.enums.ascii"):
        registered = Side.from_str("X")
        again = Side("X")
        by_code = Side(0x58000000)

    assert registered is again is by_code
    assert int(registered) == 0x58000000
    assert registered.into_str() == "X"
    assert registered.name == "X"

    # One value registers once, so the record of it is emitted once.
    assert [record.getMessage() for record in caplog.records] == [
        f"Side registered 'X' as {0x58000000}"
    ]

    # A registration is a value read back, not a declared member.
    assert [member.name for member in Side] == ["BUY", "SELL"]
    assert "X" not in Side.__members__


def test_a_registration_takes_the_generated_member_name() -> None:
    class Book(fixed_ascii(4)):
        TOP = "TOP"

    assert Book.from_str("n/a").name == AsciiEnum.member_name("n/a") == "N_A"


def test_two_names_for_one_value_are_one_member() -> None:
    class Side(fixed_ascii(4)):
        BUY = "B"
        BID = "B"

    assert Side.BID is Side.BUY
    assert [member.name for member in Side] == ["BUY"]
    assert Side.as_enum().members == {"BID": "B", "BUY": "B"}


def test_a_value_the_width_refuses_is_an_error() -> None:
    class Currency(fixed_ascii(4)):
        USD = "USD"

    with pytest.raises(ValueError, match="at most 4 bytes"):
        Currency.from_str("EURO!")
    with pytest.raises(ValueError, match="wider than the width"):
        Currency.from_code(1 << 40)
    with pytest.raises(ValueError, match="wider than the width"):
        Currency(-1)
    with pytest.raises(ValueError):
        Currency(object())


def test_a_declaration_travels_on_the_field_it_names() -> None:
    class Side(fixed_ascii(4)):
        BUY = "B"
        SELL = "S"

    field = Side.field("side", nullable=False)
    assert field.dtype == DataType.ascii(4)
    assert field.nullable is False
    assert field.ascii_enum == AsciiEnum("Side", {"BUY": "B", "SELL": "S"})
    assert field.ascii_enum.into_members(DataType.ascii(4)) == [
        ("BUY", int(Side.BUY)),
        ("SELL", int(Side.SELL)),
    ]

    # The declaration is ordinary field metadata under one reserved key, so the
    # Arrow round trip carries it and it reads back as the class that wrote it.
    assert field.get_property("field", "enum") == field.ascii_enum.into_json()
    restored = Field.from_arrow(field.into_arrow())
    recovered = AsciiCode.from_field(restored)
    assert recovered.__name__ == "Side"
    assert recovered.dtype() == DataType.ascii(4)
    assert [(member.name, int(member)) for member in recovered] == [
        (member.name, int(member)) for member in Side
    ]
    assert recovered.from_str("B").into_str() == "B"

    with pytest.raises(ValueError, match="declares no enum"):
        AsciiCode.from_field(Field("side", DataType.ascii(4)))
    with pytest.raises(ValueError, match="a fixed ASCII width or a registered code"):
        AsciiCode.from_field(
            Field("side", "utf8", metadata={"field:enum": Side.as_enum().into_json()})
        )


def test_the_registered_vocabularies_are_declared_over_their_own_datatypes() -> None:
    # Each class is the Python spelling of one registered code in the grammar,
    # over the code's own datatype rather than an ASCII width.
    for declared, base, spelling in (
        (Country, CountryCode, "country"),
        (Currency, CurrencyCode, "currency"),
        (MIC, MicCode, "mic"),
        (CFI, CfiCode, "cfi"),
    ):
        assert declared.dtype() == DataType(spelling) == base.dtype()
        assert issubclass(declared, base)
        assert declared.as_enum().name == declared.__name__
        assert declared.field(spelling).dtype == declared.dtype()

    # ISO 3166-1 is two bytes, ISO 4217 three and ISO 10962 six, so each packs
    # with none of the padding a wider width would have stored.
    assert int(Country.US) == 0x5553
    assert int(Currency.USD) == 0x555344
    assert int(MIC.XPAR) == DataType.ascii(4).ascii_packed("XPAR")
    assert int(CFI.ESVUFR) == 0x455356554652
    assert str(CFI.ESVUFR) == "ESVUFR"

    # The registries behind them keep moving, so every vocabulary is open.
    assert MIC.from_str("XLIT").into_str() == "XLIT"
    assert MIC.from_str("XLIT") is MIC("XLIT")
    assert "XLIT" not in MIC.__members__

    # A declaration reads back as the class that wrote it, over the code's
    # datatype: `currency` and `ascii(3)` are both three bytes and are not the
    # same vocabulary base.
    recovered = AsciiCode.from_field(Field.from_arrow(Currency.field("ccy").into_arrow()))
    assert recovered.__name__ == "Currency"
    assert recovered.dtype() == DataType("currency")
    assert int(recovered.USD) == int(Currency.USD)


#: A width base bound to a name, which is what an annotation can spell.
FixedAscii3 = fixed_ascii(3)


def test_an_annotation_infers_the_vocabulary_it_names() -> None:
    @scalar
    class Trade:
        ccy: Currency
        venue: MIC
        home: Country
        width: FixedAscii3

    row = Trade.field()
    declared = {child.name: child for child in row}

    assert declared["ccy"].dtype == DataType("currency")
    assert declared["ccy"].ascii_enum == Currency.as_enum()
    assert declared["venue"].dtype == DataType("mic")
    assert declared["venue"].ascii_enum == MIC.as_enum()
    assert declared["home"].dtype == DataType("country")
    assert declared["home"].ascii_enum == Country.as_enum()

    # A bare width base names no members, so it stays a plain ASCII column.
    assert declared["width"].dtype == DataType.ascii(3)
    assert declared["width"].ascii_enum is None
