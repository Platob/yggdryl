"""Python 3.14 annotation-laziness regressions without future annotations."""

import dataclasses as dc
import sys

import pytest

from yggdryl import scalar


@pytest.mark.skipif(sys.version_info < (3, 14), reason="PEP 649 is Python 3.14+")
def test_lazy_sibling_and_private_default_factory_without_future_annotations() -> None:
    @scalar
    class Earlier:
        later: Later
        __cache: list[int] = dc.field(default_factory=list)

    @scalar
    class Later:
        count: int

    assert tuple(item.name for item in dc.fields(Earlier)) == ("later",)
    assert tuple(item.name for item in Earlier.field().dtype) == (
        "later",
    )
    assert (
        Earlier.field().dtype["later"].dtype
        == Later.field().dtype
    )
    assert Earlier(Later(3)) == Earlier(later=Later(count=3))


@pytest.mark.skipif(sys.version_info < (3, 14), reason="PEP 649 is Python 3.14+")
def test_plain_private_annotation_without_future_is_not_a_dataclass_field() -> None:
    @scalar
    class Reading:
        value: int
        __scratch: str

    assert tuple(item.name for item in dc.fields(Reading)) == ("value",)
    assert tuple(item.name for item in Reading.field().dtype) == (
        "value",
    )
    assert Reading(7).value == 7
