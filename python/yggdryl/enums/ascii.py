"""Open ASCII vocabularies declared as enums over one fixed-width datatype.

A subclass of one of the six widths - `Ascii16`, `Ascii24`, `Ascii32`,
`Ascii64`, `Ascii96`, `Ascii128` - or of one of the four registered code
bases - `CountryCode`, `CurrencyCode`, `MicCode`, `CfiCode` - declares its
values as ASCII text, and a member *is*
the integer that value packs into: the value's own storage bytes read
big-endian. The code is therefore the same in every process,
is exactly what the column stores, and is what a stable hash hashes -
never a position in some column's vocabulary. The order of the integers is the
order of the text.

A width says how many bytes a value may take. A registered code says what the
value *is*, and carries that identity across Arrow, so a vocabulary declared
over `CurrencyCode` builds a `currency` column rather than three anonymous
bytes.

The vocabulary stays open: a valid value that was not declared reads back as a
member under its own packed code, registered once and announced once on the
`yggdryl.enums.ascii` logger.

A class declares itself onto a field with `field()`, which stores the members
under `field:enum`, so the enum crosses Arrow, a file, and another runtime as
ordinary field metadata; `from_field()` reads that declaration back as a class.

The worked example is in the Python extension documentation, beside the field
the declaration builds.
"""

from __future__ import annotations

import enum
import logging
from collections.abc import Iterable, Mapping
from typing import TYPE_CHECKING, Any, cast

from .._native import AsciiDictionary, AsciiEnum, DataType, Field

if TYPE_CHECKING:
    from typing import Self

#: Where a registration is announced. A value registers once, so the record of
#: a vocabulary read past its declaration is emitted once per value.
_LOGGER = logging.getLogger(__name__)

_ASCII16 = DataType("ascii16")
_ASCII24 = DataType("ascii24")
_ASCII32 = DataType("ascii32")
_ASCII64 = DataType("ascii64")
_ASCII96 = DataType("ascii96")
_ASCII128 = DataType("ascii128")
_COUNTRY = DataType("country")
_CURRENCY = DataType("currency")
_MIC = DataType("mic")
_CFI = DataType("cfi")


class AsciiCode(enum.IntEnum):
    """The shared base of the six ASCII widths and the four registered codes.

    Subclass one of those rather than this: each names the datatype the values
    store as, and this base has none.
    """

    #: The ASCII value this member codes, held once at construction.
    _text: str

    def __new__(cls, value: str | bytes) -> Self:
        dtype = cls.dtype()
        code = dtype.ascii_packed(value)
        member = int.__new__(cls, code)
        member._value_ = code
        # The stored spelling, which is the padded value read back trimmed.
        member._text = dtype.ascii_value(code)
        return member

    # -- what a member says of itself ---------------------------------------

    def into_str(self) -> str:
        """The ASCII value this member codes, without the storage padding."""

        return self._text

    def __str__(self) -> str:
        return self._text

    def __format__(self, format_spec: str) -> str:
        # An `IntEnum` renders as its integer, and the two runtimes disagree
        # about whether `__str__` or `int.__format__` decides that. Both
        # spellings render the ASCII value; `int(member)` asks for the code.
        return format(self._text, format_spec)

    def __repr__(self) -> str:
        return f"<{type(self).__name__}.{self._name_}: {self._text!r} = {int(self)}>"

    # -- parsers ------------------------------------------------------------

    @classmethod
    def from_str(cls, value: str | bytes) -> Self:
        """The member one ASCII value names, whether or not it was declared.

        Raises:
            ValueError: when `value` is not ASCII text that fits the width.
        """

        return cls.from_code(cls.dtype().ascii_packed(value))

    @classmethod
    def from_code(cls, code: int) -> Self:
        """The member one packed code names, whether or not it was declared.

        Raises:
            ValueError: when `code` is not the storage of an ASCII value of
                the width.
        """

        code = int(code)
        known = cls._value2member_map_.get(code)
        if isinstance(known, cls):
            return known
        # The width refuses a code it could not have stored, so a member is
        # only ever built for bytes this vocabulary could carry.
        value = cls.dtype().ascii_value(code)
        member = int.__new__(cls, code)
        member._name_ = AsciiDictionary.member_name(value)
        member._value_ = code
        member._text = value
        # Cached under its code, so every later read of the same value is this
        # member and the registration is announced exactly once.
        cls._value2member_map_[code] = member
        _LOGGER.info("%s registered %r as %d", cls.__name__, value, code)
        return member

    @classmethod
    def _missing_(cls, value: object) -> Self | None:
        if isinstance(value, int):
            return cls.from_code(value)
        if isinstance(value, (str, bytes)):
            return cls.from_str(value)
        return None

    # -- the declaration behind the class -----------------------------------

    @classmethod
    def dtype(cls) -> DataType:
        """The datatype a column of these values stores as."""

        raise TypeError(
            f"{cls.__name__} declares no datatype; subclass one of the widths "
            "Ascii16, Ascii24, Ascii32, Ascii64, Ascii96, Ascii128 or one of "
            "the codes CountryCode, CurrencyCode, MicCode, CfiCode"
        )

    @classmethod
    def as_enum(cls) -> AsciiEnum:
        """This class as the declaration a field stores: its declared members.

        A value read back at runtime is data, not a declaration, so only the
        members the class body names are here.
        """

        return AsciiEnum(
            cls.__name__,
            {name: member.into_str() for name, member in cls.__members__.items()},
        )

    @classmethod
    def field(
        cls,
        name: str,
        *,
        nullable: bool = True,
        metadata: Mapping[str, str] | Iterable[tuple[str, str]] | None = None,
    ) -> Field:
        """A field of this class's datatype, declaring this enum.

        Raises:
            ValueError: when `metadata` already declares an enum of its own.
        """

        field = Field(name, cls.dtype(), nullable=nullable, metadata=metadata)
        field.set_ascii_enum(cls.as_enum())
        return field

    @classmethod
    def from_field(cls, field: Field) -> type[AsciiCode]:
        """The class one field declares, over that field's own datatype.

        Raises:
            ValueError: when the field declares no enum, or its datatype is
                neither an ASCII width nor a registered code.
        """

        declared = field.ascii_enum
        if declared is None:
            raise ValueError(f"the field {field.name!r} declares no enum")
        # Keyed by the datatype id, not the width: `currency` and `ascii24`
        # are three bytes each and are not the same vocabulary base.
        base = _BASES.get(field.dtype.id)
        if base is None:
            raise ValueError(
                f"expected an ASCII width or a registered code to declare an "
                f"enum over, got {field.dtype}"
            )
        # The Enum functional API builds a class from names and values, and it
        # is spelled as a call on the base, which static typing reads as one
        # instantiation of it.
        declare: Any = base
        return cast(
            "type[AsciiCode]",
            declare(
                declared.name,
                list(declared.members.items()),
                module=cls.__module__,
            ),
        )

    @classmethod
    def into_dictionary(cls, key: object | None = None) -> AsciiDictionary:
        """The vocabulary this class names, as a dictionary over its datatype.

        A dictionary code is a position in the column it encodes, which is a
        different thing from a member: the member is the value's own bytes.
        """

        return cls.as_enum().into_dictionary(cls.dtype(), key)


class Ascii16(AsciiCode):
    """A vocabulary of values of at most two bytes, packed into 16 bits."""

    @classmethod
    def dtype(cls) -> DataType:
        return _ASCII16


class Ascii24(AsciiCode):
    """A vocabulary of values of at most three bytes, packed into 24 bits."""

    @classmethod
    def dtype(cls) -> DataType:
        return _ASCII24


class Ascii32(AsciiCode):
    """A vocabulary of values of at most four bytes, packed into an `int32`."""

    @classmethod
    def dtype(cls) -> DataType:
        return _ASCII32


class Ascii64(AsciiCode):
    """A vocabulary of values of at most eight bytes, packed into an `int64`."""

    @classmethod
    def dtype(cls) -> DataType:
        return _ASCII64


class Ascii96(AsciiCode):
    """A vocabulary of values of at most twelve bytes, packed into 96 bits.

    An ISIN is exactly this width, and its packed code needs more than an
    `int64`, which is why every code crosses as the language's own wide
    integer.
    """

    @classmethod
    def dtype(cls) -> DataType:
        return _ASCII96


class Ascii128(AsciiCode):
    """A vocabulary of values of at most sixteen bytes, packed into an `int128`."""

    @classmethod
    def dtype(cls) -> DataType:
        return _ASCII128


class CountryCode(AsciiCode):
    """A vocabulary of ISO 3166-1 alpha-2 country codes, over `country`."""

    @classmethod
    def dtype(cls) -> DataType:
        return _COUNTRY


class CurrencyCode(AsciiCode):
    """A vocabulary of ISO 4217 currency codes, over `currency`."""

    @classmethod
    def dtype(cls) -> DataType:
        return _CURRENCY


class MicCode(AsciiCode):
    """A vocabulary of ISO 10383 market identifier codes, over `mic`."""

    @classmethod
    def dtype(cls) -> DataType:
        return _MIC


class CfiCode(AsciiCode):
    """A vocabulary of ISO 10962 classifications, over `cfi`."""

    @classmethod
    def dtype(cls) -> DataType:
        return _CFI


#: The base each datatype declares its values under, keyed by datatype id.
_BASES: Mapping[str, type[AsciiCode]] = {
    "ascii16": Ascii16,
    "ascii24": Ascii24,
    "ascii32": Ascii32,
    "ascii64": Ascii64,
    "ascii96": Ascii96,
    "ascii128": Ascii128,
    "country": CountryCode,
    "currency": CurrencyCode,
    "mic": MicCode,
    "cfi": CfiCode,
}


__all__ = [
    "Ascii16",
    "Ascii24",
    "Ascii32",
    "Ascii64",
    "Ascii96",
    "Ascii128",
    "AsciiCode",
    "CfiCode",
    "CountryCode",
    "CurrencyCode",
    "MicCode",
]
