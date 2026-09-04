"""Open ASCII vocabularies declared as enums over one fixed-width datatype.

A subclass of one of the four registered code bases - `CountryCode`,
`CurrencyCode`, `MicCode`, `CfiCode` - or of the base `fixed_ascii(width)`
builds, declares its values as ASCII text, and a member *is* the integer that
value packs into: the value's own storage bytes read big-endian. The code is
therefore the same in every process, is exactly what the column stores, and is
what a stable hash hashes - never a position in some column's vocabulary. The
order of the integers is the order of the text.

Only a fixed width has a packed integer, so only a fixed width names an enum:
`DataType("ascii")` takes a value of any length and has no integer its bytes
always fit.

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
import functools
import logging
from collections.abc import Iterable, Mapping
from typing import TYPE_CHECKING, Any, cast

from .._native import AsciiEnum, DataType, Field

if TYPE_CHECKING:
    from typing import Self

#: Where a registration is announced. A value registers once, so the record of
#: a vocabulary read past its declaration is emitted once per value.
_LOGGER = logging.getLogger(__name__)

_COUNTRY = DataType("country")
_CURRENCY = DataType("currency")
_MIC = DataType("mic")
_CFI = DataType("cfi")


class AsciiCode(enum.IntEnum):
    """The shared base of the fixed widths and the four registered codes.

    Subclass `fixed_ascii(width)` or one of the codes rather than this: each
    names the datatype the values store as, and this base has none.
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
        member._name_ = AsciiEnum.member_name(value)
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
            f"{cls.__name__} declares no datatype; subclass fixed_ascii(width) "
            "or one of the codes CountryCode, CurrencyCode, MicCode, CfiCode"
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
        # Read off the datatype, not the width alone: `currency` and
        # `ascii(3)` are three bytes each and are not the same base.
        base = _base_for(field.dtype)
        if base is None:
            raise ValueError(
                f"expected a fixed ASCII width or a registered code to "
                f"declare an enum over, got {field.dtype}"
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


@functools.lru_cache(maxsize=None)
def fixed_ascii(width: int) -> type[AsciiCode]:
    """The vocabulary base for values of exactly `width` bytes.

    One class per width, built once and cached, so two declarations of the
    same width share a base and `from_field` answers that one class.

    Raises:
        ValueError: when `width` is not at least one byte, or is wider than
            the sixteen bytes a packed code holds.
    """

    dtype = DataType.ascii(width)
    # A member *is* the packed integer, so a width with no packed integer
    # names no vocabulary. Probing here refuses that width at the declaration
    # rather than at the first value.
    dtype.ascii_packed("")

    class _FixedAscii(AsciiCode):
        @classmethod
        def dtype(cls) -> DataType:
            return dtype

    _FixedAscii.__name__ = f"FixedAscii{width}"
    _FixedAscii.__qualname__ = _FixedAscii.__name__
    _FixedAscii.__doc__ = (
        f"A vocabulary of values of at most {width} bytes, packed into "
        f"{width * 8} bits."
    )
    return _FixedAscii


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


#: The base each registered code declares its values under.
_CODES: Mapping[str, type[AsciiCode]] = {
    "country": CountryCode,
    "currency": CurrencyCode,
    "mic": MicCode,
    "cfi": CfiCode,
}


def _base_for(dtype: DataType) -> type[AsciiCode] | None:
    """The base one datatype declares its vocabulary under, if it has one."""

    code = _CODES.get(dtype.id)
    if code is not None:
        return code
    # A fixed width names a vocabulary; the variable shape has no packed
    # integer, so it names none.
    width = dtype.ascii_width
    return None if width is None else fixed_ascii(width)


__all__ = [
    "AsciiCode",
    "CfiCode",
    "CountryCode",
    "CurrencyCode",
    "MicCode",
    "fixed_ascii",
]
