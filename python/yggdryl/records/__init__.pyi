from __future__ import annotations

from dataclasses import (
    KW_ONLY as KW_ONLY,
    MISSING as MISSING,
    Field as Field,
    FrozenInstanceError as FrozenInstanceError,
    InitVar as InitVar,
    asdict as asdict,
    astuple as astuple,
    dataclass as dataclass,
    field as field,
    fields as fields,
    is_dataclass as is_dataclass,
    make_dataclass as make_dataclass,
    replace as replace,
)
from collections.abc import Iterable, Iterator, Mapping
import sys
from typing import Any, Callable, Literal, TypeAlias, TypeVar, overload

if sys.version_info >= (3, 11):
    from typing import dataclass_transform
else:
    from typing_extensions import dataclass_transform

from .._native import DataType, Field as SchemaField
from .._codec import CodecFormat, Destination, Source

_T = TypeVar("_T")
_RecordT = TypeVar("_RecordT", bound="Record")
Format: TypeAlias = CodecFormat
class Record:
    __yggdryl_field__: SchemaField
    __yggdryl_fields__: tuple[SchemaField, ...]
    def to_dict(self, *, safe: bool = True) -> dict[str, Any]: ...
    @classmethod
    def from_dict(
        cls: type[_RecordT],
        values: Mapping[str, Any],
        *,
        safe: bool = True,
        errors: Literal["raise", "default"] = "raise",
    ) -> _RecordT: ...
    @classmethod
    def from_dicts(
        cls: type[_RecordT],
        values: Iterable[Mapping[str, Any]],
        *,
        safe: bool = True,
        errors: Literal["raise", "default"] = "raise",
    ) -> Iterator[_RecordT]: ...
    @classmethod
    def from_arrow_field(
        cls,
        value: object,
        *,
        class_name: str | None = None,
        module: str | None = None,
    ) -> type[Record]: ...
    @classmethod
    def from_arrow_schema(
        cls,
        value: object,
        *,
        class_name: str | None = None,
        module: str | None = None,
    ) -> type[Record]: ...
    @classmethod
    def from_arrow_record_batch(
        cls: type[_RecordT],
        batch: object,
        *,
        safe: bool = True,
        errors: Literal["raise", "default"] = "raise",
        validate_schema: bool = True,
    ) -> Iterator[_RecordT]: ...
    @classmethod
    def from_arrow_record_batch_reader(
        cls: type[_RecordT],
        reader: object,
        *,
        safe: bool = True,
        errors: Literal["raise", "default"] = "raise",
        validate_schema: bool = True,
    ) -> Iterator[_RecordT]: ...
    @classmethod
    def from_arrow_table(
        cls: type[_RecordT],
        table: object,
        *,
        safe: bool = True,
        errors: Literal["raise", "default"] = "raise",
        validate_schema: bool = True,
    ) -> Iterator[_RecordT]: ...
    @classmethod
    def from_arrow(
        cls: type[_RecordT],
        source: object,
        *,
        safe: bool = True,
        errors: Literal["raise", "default"] = "raise",
        validate_schema: bool = True,
    ) -> Iterator[_RecordT]: ...
    @classmethod
    def from_json(
        cls: type[_RecordT],
        source: Source,
        *,
        safe: bool = True,
        errors: Literal["raise", "default"] = "raise",
    ) -> _RecordT: ...
    @classmethod
    def from_yaml(
        cls: type[_RecordT],
        source: Source,
        *,
        safe: bool = True,
        errors: Literal["raise", "default"] = "raise",
    ) -> _RecordT: ...
    @classmethod
    def from_toml(
        cls: type[_RecordT],
        source: Source,
        *,
        safe: bool = True,
        errors: Literal["raise", "default"] = "raise",
    ) -> _RecordT: ...
    @classmethod
    def from_(
        cls: type[_RecordT],
        source: Source,
        *,
        format: Format | None = None,
        safe: bool = True,
        errors: Literal["raise", "default"] = "raise",
    ) -> _RecordT: ...
    @overload
    def into_json(
        self,
        destination: None = None,
        *,
        safe: bool = True,
    ) -> bytes: ...
    @overload
    def into_json(
        self,
        destination: Destination,
        *,
        safe: bool = True,
    ) -> None: ...
    @overload
    def into_yaml(
        self,
        destination: None = None,
        *,
        safe: bool = True,
    ) -> bytes: ...
    @overload
    def into_yaml(
        self,
        destination: Destination,
        *,
        safe: bool = True,
    ) -> None: ...
    @overload
    def into_toml(
        self,
        destination: None = None,
        *,
        safe: bool = True,
    ) -> bytes: ...
    @overload
    def into_toml(
        self,
        destination: Destination,
        *,
        safe: bool = True,
    ) -> None: ...
    @overload
    def into_(
        self,
        destination: None = None,
        *,
        format: Format | None = None,
        safe: bool = True,
    ) -> bytes: ...
    @overload
    def into_(
        self,
        destination: Destination,
        *,
        format: Format | None = None,
        safe: bool = True,
    ) -> None: ...
    @classmethod
    def into_arrow_field(cls) -> Any: ...
    @classmethod
    def into_arrow_schema(cls) -> Any: ...
    @classmethod
    def into_arrow_record_batch(
        cls: type[_RecordT],
        values: Iterable[_RecordT],
        *,
        safe: bool = True,
    ) -> Any: ...
    @classmethod
    def into_arrow_record_batches(
        cls: type[_RecordT],
        values: Iterable[_RecordT],
        *,
        batch_size: int = 65_536,
        safe: bool = True,
    ) -> Iterator[Any]: ...
    @classmethod
    def into_arrow_table(
        cls: type[_RecordT],
        values: Iterable[_RecordT],
        *,
        safe: bool = True,
    ) -> Any: ...
    @classmethod
    def into_arrow_record_batch_reader(
        cls: type[_RecordT],
        values: Iterable[_RecordT],
        *,
        batch_size: int = 65_536,
        safe: bool = True,
    ) -> Any: ...
    @classmethod
    def schema_field(cls) -> SchemaField: ...
    @classmethod
    def schema_fields(cls) -> tuple[SchemaField, ...]: ...

def datatype_from_pyhint(hint: object) -> DataType: ...
def field_from_pyhint(
    name: str,
    hint: object,
    metadata: Mapping[str, str] | Iterable[tuple[str, str]] | None = None,
) -> SchemaField: ...

@overload
@dataclass_transform(field_specifiers=(field, Field))
def record(cls: type[_T], /, **options: Any) -> type[_T]: ...
@overload
@dataclass_transform(field_specifiers=(field, Field))
def record(cls: None = None, /, **options: Any) -> Callable[[type[_T]], type[_T]]: ...

def from_dict(
    cls: type[_T],
    values: Mapping[str, Any],
    *,
    safe: bool = True,
    errors: Literal["raise", "default"] = "raise",
) -> _T: ...
def to_dict(value: _T, *, safe: bool = True) -> dict[str, Any]: ...
def schema_field(value: object) -> SchemaField: ...
def schema_fields(value: object) -> tuple[SchemaField, ...]: ...

__all__: list[str]
