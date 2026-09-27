"""Typing for `yggdryl.state`: the members are the native table's, listed
for static checkers; `python/tests/test_state.py` holds the list to it."""

from __future__ import annotations

import enum
from typing import Literal, TypeAlias

from ._common import MetadataInput
from ._typing import TypedField

class State(enum.IntEnum):
    UNKNOWN = 0
    PENDING = 1000
    PENDING_NEW = 1001
    QUEUED = 1002
    RECEIVED = 1003
    PENDING_VERIFICATION = 1004
    PENDING_ALLOCATION = 1005
    PENDING_APPROVAL = 1006
    ACCEPTED = 2000
    NEW = 2001
    STARTING = 2002
    SUBMITTED = 2003
    ACKNOWLEDGED = 2004
    RUNNING = 3000
    STATUS = 3001
    TRIGGERED = 3002
    ACTIVE = 3003
    UPDATED = 3004
    IN_PROGRESS = 4000
    PARTIALLY_FILLED = 4001
    TRADE = 4002
    TRADE_CORRECT = 4003
    TRADE_CANCEL = 4004
    TRADE_IN_CLEARING_HOLD = 4005
    PAUSED = 5000
    STOPPED = 5001
    SUSPENDED = 5002
    LOCKED = 5003
    DISPUTED = 5004
    INCOMPLETE = 5005
    PENDING_CANCEL = 6000
    PENDING_REPLACE = 6001
    PENDING_REVERSAL = 6002
    REPLACED = 7000
    RESTATED = 7001
    AMENDED = 7002
    RELEASED = 7003
    CALCULATED = 8000
    COMPLETE = 8001
    DONE_FOR_DAY = 8002
    FILLED = 8003
    SUCCEEDED = 8004
    TRADE_RELEASED_TO_CLEARING = 8005
    ALLOCATED = 8006
    CONFIRMED = 8007
    AFFIRMED = 8008
    VERIFIED = 8009
    CLEARED = 8010
    SETTLED = 8011
    CLAIMED = 8012
    CANCELED = 9000
    REVERSED = 9001
    REMOVED = 9002
    TERMINATED = 9003
    EXPIRED = 9500
    FAILED = 9501
    REJECTED = 9502
    TIMED_OUT = 9503
    DONT_KNOW = 9504
    MISMATCHED = 9505
    NOT_FOUND = 9506
    @property
    def description(self) -> str: ...
    @property
    def rank(self) -> int: ...
    def is_pending(self) -> bool: ...
    def is_live(self) -> bool: ...
    def is_done(self) -> bool: ...
    def is_cancelled(self) -> bool: ...
    def is_failed(self) -> bool: ...
    def is_execution(self) -> bool: ...
    @classmethod
    def from_spelling(cls, spelling: str) -> State | None: ...
    @classmethod
    def from_fix_status(cls, tag: int, code: str) -> State | None: ...
    @classmethod
    def from_fix_msgtype(cls, msgtype: str) -> State | None: ...

StateField: TypeAlias = TypedField[Literal["state"], State]

def state(name: str, *, nullable: bool = True, metadata: MetadataInput = None) -> StateField: ...

__all__ = ["State", "StateField", "state"]
