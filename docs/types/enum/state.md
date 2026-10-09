# State

What state one thing is in, from asked for to ended: a lifecycle-sorted enum of sixty-two members, stored as the `uint16` code of its member, the hundreds of the code its rank.

## Contract

| Aspect | Rule |
| --- | --- |
| Owns | `state`, `StateType`/`StateField`, the `State` enum and `Scalar::State`; the FIX readings of a state are the `yggdryl-fix` crate's free functions (`yggdryl_fix::state`), no methods of `State` |
| Validates | A member, the code of one, or a spelling one of five vocabularies names - a stored name, a FIX wire code, the specification's name, a scheduler's word, a bridge's short name - reaches one member; anything else is refused rather than stored |
| Lazy | Nothing - the member table and the vocabularies are static |
| Cached | The Arrow projection of its [`Field`](../field.md) |
| Refuses | An integer that is the code of no member, naming the code; a spelling that names no state, naming the spelling |
| Stores | `uint16` under `yggdryl.state`: the codes sort from the first state to the terminal ones in every format the column crosses |

One vocabulary over two worlds. FIX names an order's state twice - `OrdStatus(39)` says where the order stands and `ExecType(150)` says what the report is - its post-trade messages name a report's, an allocation's and a confirmation's states again, and a scheduler names a job's state in ordinary English. They are the same shape: a thing is asked for, acknowledged, it works, and it ends one of three ways. A capture and the pipeline reading it need one vocabulary rather than several and a join.

## DataType

`state` is the one spelling; the datatype is the variant, kind `enum`, and there is no shorthand constructor.

=== "Rust"

    ```rust
    use yggdryl::{DataType, DataTypeKind};

    assert_eq!(DataType::from_str("state")?, DataType::State);
    assert_eq!(DataType::State.to_string(), "state");
    assert_eq!(DataType::State.kind(), DataTypeKind::Enum);
    assert!(!DataType::State.is_code());
    assert_eq!(DataType::State.code_width(), None);
    ```

=== "Python"

    ```python
    from yggdryl import DataType

    state = DataType("state")
    assert (state.id, state.kind, state.code_width) == ("state", "enum", None)
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType } = require('yggdryl')

    const state = new DataType('state')
    assert.equal(state.id, 'state')
    assert.equal(state.kind, 'enum')
    assert.equal(state.codeWidth, null)
    ```

## Field

`StateField` is the typed marker; Python and JavaScript name the factory `state`.

=== "Rust"

    ```rust
    use yggdryl::{DataType, Field, StateField};

    let state = StateField::unit("state", false);
    assert_eq!(state.dtype(), &DataType::State);
    assert_eq!(state.to_field(), Field::new("state", DataType::State, false));
    ```

=== "Python"

    ```python
    import yggdryl
    from yggdryl import Field

    state = yggdryl.state("state", nullable=False)
    assert isinstance(state, Field)
    assert str(state.dtype) == "state"
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { fields } = require('yggdryl')

    const state = fields.state('state', { nullable: false })
    assert.equal(state.dtype.toString(), 'state')
    ```

## Scalar

The value is the member, whichever vocabulary named it: `PARTIALLY_FILLED` for FIX's `1`, its `PartiallyFilled`, a bridge's `PartFill`, the stored name itself, or its code `4001`. Rust holds the `State` member; Python the member of the `yggdryl.State` `IntEnum`, which is the integer it stores and renders as its name; JavaScript the member's name, with `State` mapping every name to its code.

=== "Rust"

    ```rust
    use yggdryl::{DataType, Scalar, State};

    let partial = DataType::State.scalar("PARTIALLY_FILLED")?;
    assert_eq!(partial, Scalar::State(State::PartiallyFilled));
    assert_eq!(partial.kind(), "state");
    assert_eq!(State::PartiallyFilled.code(), 4001);

    // Five vocabularies and the code reach one value.
    assert_eq!(DataType::State.scalar("1")?, partial);
    assert_eq!(DataType::State.scalar("PartiallyFilled")?, partial);
    assert_eq!(DataType::State.scalar("PartFill")?, partial);
    assert_eq!(DataType::State.scalar(4001_i32)?, partial);
    assert_eq!(DataType::State.scalar("running")?, Scalar::State(State::Running));

    // A spelling nothing publishes, or the code of no member, answers nothing
    // rather than a guess.
    assert!(DataType::State.scalar("nonsense").is_err());
    assert!(DataType::State.scalar(7_i32).is_err());
    ```

=== "Python"

    ```python
    import pytest

    from yggdryl import DataType, State

    state = DataType("state")
    assert state.scalar("1").as_py() is State.PARTIALLY_FILLED
    assert state.scalar("PartiallyFilled").as_py() is State.PARTIALLY_FILLED
    assert state.scalar(4001).as_py() is State.PARTIALLY_FILLED
    assert state.scalar("running").as_py() is State.RUNNING
    assert state.scalar("1").kind == "state"

    # A member is the integer it stores and reads as its name.
    assert State.PARTIALLY_FILLED == 4001 and str(State.PARTIALLY_FILLED) == "PARTIALLY_FILLED"

    with pytest.raises(ValueError, match="state"):
        state.scalar("nonsense")
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType, State } = require('yggdryl')

    const state = new DataType('state')
    assert.equal(state.scalar('1').asJs(), 'PARTIALLY_FILLED')
    assert.equal(state.scalar('running').asJs(), 'RUNNING')
    assert.equal(State.PARTIALLY_FILLED, 4001)
    assert.throws(() => state.scalar('nonsense'), /state/)
    ```

## Arrow storage

`UInt16` under `yggdryl.state`. The column is one value buffer of codes, and because the codes sort by lifecycle a Parquet row group's bounds, an Iceberg predicate and an external `ORDER BY` already read in lifecycle order. Text entering a `state` column is read as a spelling and an integer of any width, signed or unsigned, as a code, each refused - or null under `safe` in a nullable column - where it names no member; a `state` column cast to text answers each member's name, and cast to an integer its code.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{Array, ArrayRef, StringArray, UInt16Array};
    use arrow_schema::DataType as ArrowDataType;
    use yggdryl::{ArrowCastOptions, DataType, Field, Serie};

    let state = Field::new("state", DataType::State, false);
    let arrow = state.clone().into_arrow_field()?;
    assert_eq!(arrow.data_type(), &ArrowDataType::UInt16);
    assert_eq!(arrow.metadata()["ARROW:extension:name"], "yggdryl.state");
    assert_eq!(Field::from_arrow_field(&arrow)?, state);

    // Names land as the codes their members store.
    let names: ArrayRef = Arc::new(StringArray::from(vec!["FILLED", "NEW", "PartFill"]));
    let stored = Serie::from_arrow_array(Some(&state), names, ArrowCastOptions::new())?
        .into_arrow_array()
        .expect("a column, not a run");
    let codes = stored.as_any().downcast_ref::<UInt16Array>().expect("uint16 codes");
    assert_eq!(codes.values().to_vec(), [8003, 2001, 4001]);
    ```

=== "Python"

    ```python
    import pyarrow as pa

    from yggdryl import Field, Serie, State

    state = Field("state", "state")
    arrow_field = state.into_arrow()
    assert arrow_field.type.storage_type == pa.uint16()
    assert arrow_field.type.extension_name == "yggdryl.state"
    assert Field.from_arrow(arrow_field) == state

    # Names land as members, and members sort by lifecycle.
    source = pa.array(["FILLED", "NEW", "PARTIALLY_FILLED"])
    stored = Serie.from_arrow_array(source, state, safe=False)
    assert sorted(stored.as_py()) == [State.NEW, State.PARTIALLY_FILLED, State.FILLED]
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const arrow = require('apache-arrow')
    const { Serie, State, fields } = require('yggdryl')

    const utf8 = (values) => arrow.vectorFromArray(values, new arrow.Utf8())
    const stored = Serie.fromArrowArray(utf8(['FILLED', 'NEW']), fields.state('state'))
    assert.deepEqual([...stored.intoArrowArray()], [State.FILLED, State.NEW])
    ```

## The code is the rank

A member's code is its **rank times one hundred plus its place in the rank**. The rank is what makes the stored integers sort from the first state to the terminal ones, and that matters because most things that sort a column are not this crate: a Parquet row group's bounds, an external sort, an `ORDER BY` in whatever reads the file. Sorting by name would put `CANCELED` before `NEW`. A state added to a rank takes the next free number of it, so nothing already stored moves.

| rank | codes | meaning | members |
| ---: | --- | --- | --- |
| `0` | `0` | stated, but not a state anything reached | `UNKNOWN` |
| `10` | `1000`-`1099` | asked for, not yet acknowledged | `PENDING`, `PENDING_NEW`, `QUEUED`, `RECEIVED` |
| `20` | `2000`-`2099` | acknowledged, not yet working | `ACCEPTED`, `NEW`, `STARTING`, `SUBMITTED`, `ACKNOWLEDGED` |
| `30` | `3000`-`3099` | working | `RUNNING`, `STATUS`, `TRIGGERED`, `ACTIVE`, `UPDATED` |
| `40` | `4000`-`4099` | working, and something has happened - or acknowledged and awaiting its next step | `IN_PROGRESS`, `PARTIALLY_FILLED`, `TRADE`, `TRADE_CORRECT`, `TRADE_CANCEL`, `TRADE_IN_CLEARING_HOLD`, `PENDING_VERIFICATION`, `PENDING_ALLOCATION`, `PENDING_APPROVAL` |
| `50` | `5000`-`5099` | halted, and able to resume | `PAUSED`, `STOPPED`, `SUSPENDED`, `LOCKED`, `DISPUTED`, `INCOMPLETE` |
| `60` | `6000`-`6099` | a change is outstanding | `PENDING_CANCEL`, `PENDING_REPLACE`, `PENDING_REVERSAL` |
| `70` | `7000`-`7099` | changed, and the new thing carries on | `REPLACED`, `RESTATED`, `AMENDED`, `RELEASED` |
| `80` | `8000`-`8999` | ended, having done what was asked | `CALCULATED`, `COMPLETE`, `DONE_FOR_DAY`, `FILLED`, `SUCCEEDED`, `TRADE_RELEASED_TO_CLEARING`, `ALLOCATED`, `CONFIRMED`, `AFFIRMED`, `VERIFIED`, `CLEARED`, `SETTLED`, `CLAIMED`, `APPROVED` |
| `90` | `9000`-`9499` | ended, because someone stopped it | `CANCELED`, `REVERSED`, `REMOVED`, `TERMINATED` |
| `95` | `9500`-`9999` | ended, because it could not be done | `EXPIRED`, `FAILED`, `REJECTED`, `TIMED_OUT`, `DONT_KNOW`, `MISMATCHED`, `NOT_FOUND` |

The three endings are ranked apart deliberately: "did it finish" and "did it work" are different questions, and one terminal rank would answer neither without reading the name. Each ending owns a band, and `is_pending` (rank `10`), `is_live` (below `80`), `is_done`, `is_cancelled` and `is_failed` read the band. `is_execution` is deliberately exact rather than ranked: only `PARTIALLY_FILLED`, `TRADE` and `FILLED` report an execution; a trade correction, cancellation, hold or release refers to an earlier one. Every member states what it means (`description`).

=== "Rust"

    ```rust
    use yggdryl::State;

    // The codes sort by lifecycle, which is the whole reason the rank leads.
    let mut held = [State::Filled, State::New, State::Rejected, State::PartiallyFilled];
    held.sort_unstable();
    assert_eq!(held, [State::New, State::PartiallyFilled, State::Filled, State::Rejected]);
    assert!(State::ALL.windows(2).all(|pair| pair[0].code() < pair[1].code()));

    // The rank is the code's hundreds, and the three endings are told apart
    // by band without reading a name.
    assert_eq!(State::Filled.rank(), 80);
    assert!(State::PendingNew.is_pending() && State::New.is_live());
    assert!(State::PartiallyFilled.is_execution());
    assert!(State::Filled.is_done());
    assert!(State::Canceled.is_cancelled());
    assert!(State::Rejected.is_failed());
    assert_eq!(State::from_code(9500), Some(State::Expired));
    assert_eq!(State::from_name("EXPIRED"), Some(State::Expired));
    ```

=== "Python"

    ```python
    from yggdryl import State

    held = sorted([State.FILLED, State.NEW, State.REJECTED, State.PARTIALLY_FILLED])
    assert held == [State.NEW, State.PARTIALLY_FILLED, State.FILLED, State.REJECTED]
    assert [int(member) for member in State] == sorted(int(member) for member in State)

    assert State.FILLED.rank == 80
    assert State.PENDING_NEW.is_pending() and State.NEW.is_live()
    assert State.PARTIALLY_FILLED.is_execution()
    assert State.FILLED.is_done() and State.CANCELED.is_cancelled() and State.REJECTED.is_failed()
    assert State(9500) is State.EXPIRED and State["EXPIRED"] is State.EXPIRED
    assert State.EXPIRED.description
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { State } = require('yggdryl')

    const codes = Object.values(State)
    assert.deepEqual(codes, [...codes].sort((left, right) => left - right))
    assert.ok(State.NEW < State.PARTIALLY_FILLED && State.FILLED < State.REJECTED)
    assert.equal(Math.floor(State.FILLED / 100), 80)
    ```

## FIX status fields, one vocabulary

A FIX message states its state in whichever status field its kind answers a request by. `yggdryl_fix::state::from_status(tag, code)` reads each under its own code set, and a [FIX message](../../fix/capture.md) takes the first of `yggdryl_fix::state::STATUS_TAGS` it states - `OrdStatus(39)`, `ExecType(150)`, `ExecAckStatus(1036)`, `TrdRptStatus(939)`, `QuoteStatus(297)`, `AllocStatus(87)`, `ConfirmStatus(665)`, `AffirmStatus(940)`, `MassActionResponse(1375)`, `MassCancelResponse(531)` - else the state its message type asks for by being the message it is (`yggdryl_fix::state::from_msgtype`): a new order is `PENDING_NEW`, a cancel request `PENDING_CANCEL`, a reject `REJECTED`. The three are free functions of the FIX crate, so `State` holds no FIX table: it keeps the `OrdStatus(39)`/`ExecType(150)` wire-code table its own `from_spelling` reads, which `from_status` reads for those two tags. Rust; Python keeps the classmethods `State.from_fix_status` and `State.from_fix_msgtype`, redirecting to them, and JavaScript has none.

`OrdStatus(39)` and `ExecType(150)` share their letters and not always their meaning - `D` is Restated in one and AcceptedForBidding in the other - so a [FIX column](../../fix/capture.md) reads a code through the name its own field's [code set](../../fix/registry.md#a-field-names-the-code-set-it-reads-by) gives it before it reads the letter: `150=D` is `RESTATED` and `39=D` is `ACCEPTED`. The FIX dictionary holds the members themselves as the crate's own `statecodeset`, each code's value the integer a column stores and its name the stored name.

=== "Rust"

    ```rust
    use yggdryl::State;
    use yggdryl_fix::state;
    yggdryl_fix::install()?;

    assert_eq!(state::from_status(39, "1"), Some(State::PartiallyFilled));
    assert_eq!(state::from_status(1036, "1"), Some(State::Acknowledged));
    assert_eq!(state::from_status(87, "0"), Some(State::Allocated));
    assert_eq!(state::from_msgtype("D"), Some(State::PendingNew));
    assert_eq!(state::from_msgtype("8"), None);
    ```

=== "Python"

    ```python
    from yggdryl import State

    assert State.from_fix_status(39, "1") is State.PARTIALLY_FILLED
    assert State.from_fix_status(1036, "1") is State.ACKNOWLEDGED
    assert State.from_fix_msgtype("D") is State.PENDING_NEW
    assert State.from_fix_msgtype("8") is None
    assert State.from_spelling("PartFill") is State.PARTIALLY_FILLED
    ```

## The further along stands

A state that reached none - `UNKNOWN` - takes the other, and otherwise the state further along by rank stands, whichever side it is on. A trade report awaiting its verification, an allocation awaiting its making and a give-up awaiting its approval were each acknowledged first, so they rank `40`: past their acknowledgement, and below their dispute or incompleteness, a pending cancel and their verification, allocation or approval, so the fold keeps the answer whichever order it arrives in. `APPROVED` is reached by spelling alone: FIX states an approved give-up as `AllocStatus(87)` `0`, which reads `ALLOCATED`. Rust only.

```rust
use yggdryl::State;
use yggdryl_fix::state;
yggdryl_fix::install()?;

assert_eq!(State::unknown(), State::Unknown);
assert_eq!(State::Unknown.merge_with(State::New), State::New);
assert_eq!(State::New.merge_with(State::Filled), State::Filled);
assert_eq!(State::Filled.merge_with(State::New), State::Filled);

// Awaiting a verification ranks past the acceptance and below the answers.
assert_eq!(state::from_status(939, "8"), Some(State::PendingVerification));
assert_eq!(State::PendingVerification.code(), 4006);
assert_eq!(State::Accepted.merge_with(State::PendingVerification), State::PendingVerification);
assert_eq!(State::Disputed.merge_with(State::PendingVerification), State::Disputed);
assert_eq!(State::PendingApproval.merge_with(State::Approved), State::Approved);
assert_eq!(State::from_spelling("approved"), Some(State::Approved));
```

## Stated anew over a live one

`UPDATED` (`3004`) is the working band's member for a thing stated anew while it is live. `is_new_like` answers which states something can be stated anew over: acknowledged (rank `20`) or working (rank `30`), or one of the changes that carry on - `UPDATED`, `REPLACED`, `RESTATED`, `AMENDED`. Never the pending band, because a `NEW` after a `PENDING_NEW` is the first acknowledgement rather than a restatement, and never a state that progressed or ended. The [lifecycle walk](../../graph/event.md#lifecycle-walk) reads it: a statement of `NEW` following a live element whose state is new-like is that element `UPDATED`, and later progress folds over it by rank as over any state. `is_new_like` is Rust only.

=== "Rust"

    ```rust
    use yggdryl::State;

    assert_eq!(State::Updated.code(), 3004);
    assert_eq!(State::Updated.rank(), 30);
    assert!(State::Updated.is_live());

    // Acknowledged, working and carrying on can be stated anew over ...
    for state in [State::New, State::Active, State::Updated, State::Replaced, State::Restated] {
        assert!(state.is_new_like(), "{state}");
    }
    // ... asked for, progressed and ended cannot.
    for state in [State::PendingNew, State::PartiallyFilled, State::Filled, State::Canceled] {
        assert!(!state.is_new_like(), "{state}");
    }

    // Later progress folds over it by rank, and a `NEW` does not undo it.
    assert_eq!(State::Updated.merge_with(State::PartiallyFilled), State::PartiallyFilled);
    assert_eq!(State::Updated.merge_with(State::New), State::Updated);
    ```

=== "Python"

    ```python
    from yggdryl import State

    assert State.UPDATED == 3004 and State.UPDATED.rank == 30
    assert State.UPDATED.is_live()
    assert State.from_spelling("UPDATED") is State.UPDATED
    assert sorted([State.PARTIALLY_FILLED, State.UPDATED, State.NEW]) == [
        State.NEW,
        State.UPDATED,
        State.PARTIALLY_FILLED,
    ]
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType, State } = require('yggdryl')

    assert.equal(State.UPDATED, 3004)
    assert.equal(Math.floor(State.UPDATED / 100), 30)
    assert.ok(State.NEW < State.UPDATED && State.UPDATED < State.PARTIALLY_FILLED)
    assert.equal(new DataType('state').scalar('UPDATED').asJs(), 'UPDATED')
    ```

## Edges

- A spelling that names no state, or an integer that is the code of none -> refused naming `state`, never stored; a column typed `state` therefore holds members only, and a value that names none leaves a nullable column null under `safe`.
- A wire code never folds: `A` is `PENDING_NEW` and `a` names no state, because they are different FIX codes and a folded lookup would answer the wrong state for one of them.
- A name folds: `DoneForDay`, `done_for_day`, `DONE FOR DAY` and a bridge's `DoneDay` are one spelling, `DONE_FOR_DAY`.
- A stored code is an integer, never text: `"2001"` is no spelling, `2001` is `NEW`.
- In an expression a text constant meets a `state` column as the member it spells and an integer as the code it stores: `state = 'FILLED'`, `state in ('NEW', 'PartFill')`, `state < 8000` (every state still live) and `state >= 9000` (cancelled or failed) all compare by lifecycle; a state beside an integer column merges to `int32`, which holds every code.
- A Hive partition over a `state` column is named by the member, `state=FILLED`, the spelling the column reads back.
- The default value is `UNKNOWN`, code `0`: a stated value, not an absence.
- JSON, TOML, YAML and XML write a state as its name, the [value stream](../value-stream.md) and a digest feed its four-byte little-endian code, and Iceberg stores it as an `int`.

## Commands

=== "Rust"

    ```bash
    cargo test --manifest-path rust/Cargo.toml -p yggdryl --test root -- state::
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/test_state.py -q
    ```

=== "JavaScript"

    ```bash
    node --test node/tests/state.test.js
    ```
