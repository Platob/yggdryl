"""``State``, the lifecycle-sorted ``int32`` enum: `python/yggdryl/state.py` and
`python/src/state.rs`."""

from __future__ import annotations

import pickle

import pyarrow as pa  # type: ignore[import-untyped]
import pytest

import yggdryl
from yggdryl import DataType, Field, Scalar, State


def test_the_members_are_the_cores_in_code_order() -> None:
    codes = [int(state) for state in State]
    assert codes == sorted(codes)
    assert len(codes) == len(set(codes))
    assert State.UNKNOWN == 0 and State.NEW == 2001 and State.EXPIRED == 9500
    # A member's hundreds are its rank, and a member is its stored name.
    for state in State:
        assert state.rank == int(state) // 100
        assert str(state) == state.name
        assert f"{state}" == state.name
        assert state.description


def test_each_band_is_one_question() -> None:
    assert State.PENDING_NEW.is_pending() and State.PENDING_NEW.is_live()
    assert State.UNKNOWN.is_live() and not State.UNKNOWN.is_pending()
    assert State.PARTIALLY_FILLED.is_live() and State.PARTIALLY_FILLED.is_execution()
    assert State.FILLED.is_done() and not State.FILLED.is_live()
    assert State.CANCELED.is_cancelled() and not State.CANCELED.is_failed()
    assert State.REJECTED.is_failed() and not State.REJECTED.is_done()
    for state in State:
        terminal = state.is_done() + state.is_cancelled() + state.is_failed()
        assert terminal == (0 if state.is_live() else 1), state


def test_a_spelling_a_status_and_a_message_type_read_to_one_member() -> None:
    assert State.from_spelling("NEW") is State.NEW
    assert State.from_spelling("0") is State.NEW
    assert State.from_spelling("PartiallyFilled") is State.PARTIALLY_FILLED
    assert State.from_spelling("not a state") is None
    assert State.from_fix_status(39, "8") is State.REJECTED
    assert State.from_fix_status(150, "F") is State.TRADE
    assert State.from_fix_status(55, "8") is None
    assert State.from_fix_msgtype("D") is State.PENDING_NEW
    assert State.from_fix_msgtype("F") is State.PENDING_CANCEL
    assert State.from_fix_msgtype("0") is None


def test_the_datatype_stores_the_code_and_reads_back_the_member() -> None:
    field = yggdryl.state("state", nullable=False)
    assert field.dtype == DataType("state")
    for given in ("NEW", 2001, State.NEW):
        value = DataType("state").scalar(given)
        assert value.as_py() is State.NEW
    with pytest.raises(ValueError, match="the code of a state"):
        DataType("state").scalar(7)
    with pytest.raises(ValueError):
        DataType("state").scalar("not a state")
    assert pickle.loads(pickle.dumps(Scalar.from_(State.FILLED))).as_py() is State.FILLED


def test_an_arrow_column_is_int32_under_its_extension() -> None:
    arrow = Field("state", "state").into_arrow()
    assert arrow.type == pa.int32()
    assert arrow.metadata[b"ARROW:extension:name"] == b"yggdryl.state"
