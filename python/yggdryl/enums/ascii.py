"""Open ASCII vocabularies declared as enums over one fixed width.

A subclass of `Ascii32`, `Ascii64`, or `Ascii128` declares its values as ASCII
text, and a member *is* the integer that value packs into: the value's own
storage bytes read big-endian. The code is therefore the same in every process,
is exactly what an ASCII column stores, and is what a stable hash hashes -
never a position in some column's vocabulary. The order of the integers is the
order of the text.

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

_ASCII32 = DataType("ascii32")
_ASCII64 = DataType("ascii64")
_ASCII128 = DataType("ascii128")


class AsciiCode(enum.IntEnum):
    """The shared base of the three ASCII widths.

    Subclass one of the widths rather than this: a width names the datatype the
    values store as, and this base has none.
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
        """The ASCII width a column of these values stores as."""

        raise TypeError(
            f"{cls.__name__} declares no ASCII width; "
            "subclass Ascii32, Ascii64, or Ascii128"
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
        """A field of this class's ASCII width, declaring this enum.

        Raises:
            ValueError: when `metadata` already declares an enum of its own.
        """

        field = Field(name, cls.dtype(), nullable=nullable, metadata=metadata)
        field.set_ascii_enum(cls.as_enum())
        return field

    @classmethod
    def from_field(cls, field: Field) -> type[AsciiCode]:
        """The class one field declares, over that field's own ASCII width.

        Raises:
            ValueError: when the field declares no enum, or its datatype is
                not an ASCII width.
        """

        declared = field.ascii_enum
        if declared is None:
            raise ValueError(f"the field {field.name!r} declares no enum")
        width = _WIDTHS.get(field.dtype.ascii_width)
        if width is None:
            raise ValueError(
                f"expected an ASCII width to declare an enum over, "
                f"got {field.dtype}"
            )
        # The Enum functional API builds a class from names and values, and it
        # is spelled as a call on the base, which static typing reads as one
        # instantiation of it.
        declare: Any = width
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
        """The vocabulary this class names, as a dictionary over its width.

        A dictionary code is a position in the column it encodes, which is a
        different thing from a member: the member is the value's own bytes.
        """

        return cls.as_enum().into_dictionary(cls.dtype(), key)


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


class Ascii128(AsciiCode):
    """A vocabulary of values of at most sixteen bytes, packed into an `int128`."""

    @classmethod
    def dtype(cls) -> DataType:
        return _ASCII128


#: The width base each storage width declares its values under.
_WIDTHS: Mapping[int | None, type[AsciiCode]] = {
    4: Ascii32,
    8: Ascii64,
    16: Ascii128,
}


__all__ = [
    "Ascii32",
    "Ascii64",
    "Ascii128",
    "AsciiCode",
]
