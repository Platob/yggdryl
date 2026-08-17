"""Static typed views over the one native :class:`yggdryl.Field` value."""

from __future__ import annotations

from typing import TYPE_CHECKING, Generic, TypeVar

from .._native import DataType, Field

_KindT = TypeVar("_KindT", bound=str)
_ValueT = TypeVar("_ValueT")

if TYPE_CHECKING:

    class TypedDataType(DataType, Generic[_KindT, _ValueT]):
        """A static datatype-kind view; runtime values remain ``DataType``."""

        # The parameter carries the exact identifier, not the coarse kind:
        # ``int32`` has the id ``int32`` but the kind ``integer``, so only
        # ``id`` can be narrowed to the literal a typed alias names.
        @property
        def id(self) -> _KindT: ...

        def default_pyvalue(self) -> _ValueT: ...

        def default_pyhint(self) -> object: ...

    class TypedField(Field, Generic[_KindT, _ValueT]):
        """A static field-kind view; runtime values remain ``Field``."""

        @property
        def data_type(self) -> TypedDataType[_KindT, _ValueT]: ...

        def default_pyvalue(self) -> _ValueT | None: ...

        def default_pyhint(self) -> object: ...

else:
    # Do not create binding-side schema state or wrapper identities. These
    # aliases make isinstance(value, TypedField) useful at runtime too.
    TypedDataType = DataType
    TypedField = Field


__all__ = ["TypedDataType", "TypedField"]
