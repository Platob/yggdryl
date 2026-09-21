# State

What state one thing is in: two decimal digits of rank then a name, so the stored bytes sort from the first state to the terminal ones.

## Contract

| Aspect | Rule |
| --- | --- |
| Owns | `state`, `StateType`/`StateField`, the `State` value and `Scalar::State` |
| Validates | By spelling: a FIX wire code, the specification's name, a scheduler's word or a stored value reaches one ranked value; anything else is refused rather than stored |
| Lazy | Nothing - the tables are static and the lookup is a scan of bounded bytes |
| Cached | The Arrow projection of its [`Field`](../field.md) |
| Refuses | A spelling that names no state, naming `state`; an eleventh stored byte, naming the width |

One vocabulary over two worlds. FIX names an order's state twice - `OrdStatus(39)` says where the order stands and `ExecType(150)` says what the report is - and a scheduler names a job's state in ordinary English. They are the same shape, so a capture and the pipeline reading it need one vocabulary rather than two and a join.

## DataType

`state` is the one spelling; the datatype is the variant, and there is no shorthand constructor.

=== "Rust"

    ```rust
    use yggdryl::{DataType, DataTypeKind};

    assert_eq!(DataType::from_str("state")?, DataType::State);
    assert_eq!(DataType::State.to_string(), "state");
    assert_eq!(DataType::State.kind(), DataTypeKind::Code);
    assert_eq!(DataType::State.code_name(), Some("state"));
    assert_eq!(DataType::State.code_width(), Some(10));
    assert_eq!(DataType::State.fixed_byte_width(), None);
    ```

=== "Python"

    ```python
    from yggdryl import DataType

    state = DataType("state")
    assert (state.id, state.code_width, state.kind) == ("state", 10, "code")
    assert state.fixed_byte_width is None
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType } = require('yggdryl')

    const state = new DataType('state')
    assert.equal(state.id, 'state')
    assert.equal(state.kind, 'code')
    assert.equal(state.codeWidth, 10)
    assert.equal(state.fixedByteWidth, null)
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

The value is the ranked spelling, whichever vocabulary named it: `40PARTFILL` for FIX's `1`, its `PartiallyFilled`, a bridge's `PartFill`, or the stored value itself.

=== "Rust"

    ```rust
    use yggdryl::{DataType, Scalar, State};

    let partial = DataType::State.scalar("40PARTFILL")?;
    assert_eq!(partial, Scalar::State(State::new("40PARTFILL")?));
    assert_eq!(partial.kind(), "state");

    // Four vocabularies reach one value.
    assert_eq!(DataType::State.scalar("1")?, partial);
    assert_eq!(DataType::State.scalar("PartiallyFilled")?, partial);
    assert_eq!(DataType::State.scalar("PartFill")?, partial);
    assert_eq!(DataType::State.scalar("running")?.as_str(), Some("30RUNNING"));

    // A spelling nothing publishes answers nothing rather than a guess.
    assert!(DataType::State.scalar("nonsense").is_err());
    ```

=== "Python"

    ```python
    import pytest

    from yggdryl import DataType

    state = DataType("state")
    assert state.scalar("1").as_py() == "40PARTFILL"
    assert state.scalar("PartiallyFilled").as_py() == "40PARTFILL"
    assert state.scalar("running").as_py() == "30RUNNING"
    assert state.scalar("1").kind == "state"

    with pytest.raises(ValueError, match="state"):
        state.scalar("nonsense")
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType } = require('yggdryl')

    const state = new DataType('state')
    assert.equal(state.scalar('1').asJs(), '40PARTFILL')
    assert.equal(state.scalar('running').asJs(), '30RUNNING')
    assert.throws(() => state.scalar('nonsense'), /state/)
    ```

## Arrow storage

`Utf8` under `yggdryl.state` ([Codes](index.md#arrow-storage)). The rank leads the stored bytes, so a Parquet row group's bounds and an external `ORDER BY` already sort by lifecycle.

=== "Rust"

    ```rust
    use arrow_schema::DataType as ArrowDataType;
    use yggdryl::{DataType, Field};

    let state = Field::new("state", DataType::State, false);
    let arrow = state.clone().into_arrow_field()?;
    assert_eq!(arrow.data_type(), &ArrowDataType::Utf8);
    assert_eq!(arrow.metadata()["ARROW:extension:name"], "yggdryl.state");
    assert_eq!(Field::from_arrow_field(&arrow)?, state);
    ```

=== "Python"

    ```python
    import pyarrow as pa

    from yggdryl import Field

    state = Field("state", "state")
    arrow_field = state.into_arrow()
    assert arrow_field.type == pa.string()
    assert arrow_field.metadata[b"ARROW:extension:name"] == b"yggdryl.state"
    assert Field.from_arrow(arrow_field) == state

    # The stored bytes sort by lifecycle, with nothing but ASCII order needed.
    stored = state.cast_arrow_array(pa.array(["80FILLED", "20NEW", "40PARTFILL"]), safe=False)
    assert sorted(stored.to_pylist()) == ["20NEW", "40PARTFILL", "80FILLED"]
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const arrow = require('apache-arrow')
    const { fields } = require('yggdryl')

    const utf8 = (values) => arrow.vectorFromArray(values, new arrow.Utf8())
    const stored = [...fields.state('state').castArrowArray(utf8(['80FILLED', '20NEW']))]
    assert.deepEqual(stored.sort(), ['20NEW', '80FILLED'])
    ```

## The rank leads

A value is **two decimal digits of rank then a name**, ten US-ASCII bytes. The rank is what makes the stored bytes sort from the first state to the terminal ones, and that matters because most things that sort a column are not this crate: a Parquet row group's bounds, an external sort, an `ORDER BY` in whatever reads the file. Sorting by name would put `CANCELED` before `NEW`.

Ranks run `00`-`99`. Every shipped state sits on a round rank, and the digits between two of them - `01`-`09`, `11`-`19`, and so on - are the placeholders a state that belongs between two ranks takes, so adding one moves nothing already stored.

| rank | meaning | members |
| --- | --- | --- |
| `00` | stated, but not a state anything reached | `00UNKNOWN` |
| `10` | asked for, not yet acknowledged | `10PENDING`, `10PENDNEW`, `10QUEUED` |
| `20` | acknowledged, not yet working | `20ACCEPTED`, `20NEW`, `20STARTING`, `20SUBMITTD` |
| `30` | working | `30RUNNING`, `30STATUS`, `30TRIGGER` |
| `40` | working, and something has happened | `40INPROGR`, `40PARTFILL`, `40TRADE`, `40TRDCORR`, `40TRDCXL`, `40TRDHOLD` |
| `50` | halted, and able to resume | `50PAUSED`, `50STOPPED`, `50SUSPEND` |
| `60` | a change is outstanding | `60PENDCXL`, `60PENDRPL` |
| `70` | changed, and the new thing carries on | `70REPLACED`, `70RESTATED` |
| `80` | ended, having done what was asked | `80CALCULAT`, `80COMPLETE`, `80DONEDAY`, `80FILLED`, `80SUCCESS`, `80TRDRELS` |
| `90` | ended, because someone stopped it | `90CANCELED` |
| `95` | ended, because it could not be done | `95EXPIRED`, `95FAILED`, `95REJECTED`, `95TIMEOUT` |

The three endings are ranked apart deliberately: "did it finish" and "did it work" are different questions, and one terminal rank would answer neither without reading the name. Each ending owns a band - `80`-`89` done, `90`-`94` cancelled, `95`-`99` failed - and `State::is_live` (below `80`), `is_done`, `is_cancelled` and `is_failed` read the band, so a placeholder inside one answers as its ending does. `State::rank` answers the two digits as the number they spell. Rust only.

```rust
use yggdryl::State;

// Four vocabularies reach one value: the wire code an ExecutionReport
// carries, the specification's name for it, a scheduler's word, and the
// short name a FIX bridge logs.
assert_eq!(State::from_spelling("1").unwrap().as_str(), "40PARTFILL");
assert_eq!(State::from_spelling("PartiallyFilled").unwrap().as_str(), "40PARTFILL");
assert_eq!(State::from_spelling("running").unwrap().as_str(), "30RUNNING");
assert_eq!(State::from_spelling("PartFill").unwrap().as_str(), "40PARTFILL");

// The stored bytes sort by lifecycle, which is the whole reason the rank
// leads - nothing but ASCII order is needed to read it back.
let mut held = ["80FILLED", "20NEW", "95REJECTED", "40PARTFILL"];
held.sort_unstable();
assert_eq!(held, ["20NEW", "40PARTFILL", "80FILLED", "95REJECTED"]);

// The rank is a number, and the three endings are told apart by band
// without reading a name.
assert_eq!(State::from_spelling("Filled").unwrap().rank(), Some(80));
assert!(State::from_spelling("New").unwrap().is_live());
assert!(State::from_spelling("Filled").unwrap().is_done());
assert!(State::from_spelling("Canceled").unwrap().is_cancelled());
assert!(State::from_spelling("Rejected").unwrap().is_failed());
```

## Two FIX code sets, one vocabulary

`OrdStatus(39)` and `ExecType(150)` are named [code sets](../../fix/registry.md#a-field-names-the-code-set-it-reads-by) the dictionary holds, `ordstatuscodeset` and `exectypecodeset`. They share their letters and not always their meaning - `D` is Restated in one and AcceptedForBidding in the other - so a [FIX column](../../fix/capture.md) reads a code through the name its own field's set gives it, before it reads the letter: `150=D` is `70RESTATED` and `39=D` is `20ACCEPTED`.

The two sets agree on every value they share, which is why one table answers both: `0` is New in each, `1` PartiallyFilled, `2` Filled. Where only `ExecType` defines a value - `F` Trade, `L` Triggered - the state is what that report says the order is doing.

## The further along stands

A state that reached none - rank `00` - takes the other, and otherwise the state further along stands, whichever side it is on ([`merge_with`](index.md#the-code-family-value)). Rust only.

```rust
use yggdryl::{CodeValue, State};

assert_eq!(State::unknown().as_str(), "00UNKNOWN");
assert_eq!(State::unknown().merge_with(&State::read("New")?), State::read("New")?);
assert_eq!(State::read("New")?.merge_with(&State::read("Filled")?), State::read("Filled")?);
assert_eq!(State::read("Filled")?.merge_with(&State::read("New")?), State::read("Filled")?);
```

## Edges

- A spelling that names no state -> refused naming `state`, never stored; a column typed `state` therefore holds ranked values only, and text that names none leaves the column null under `safe`.
- A wire code never folds: `A` is `10PENDNEW` and `a` names no state, because they are different FIX codes and a folded lookup would answer the wrong state for one of them.
- A name folds: `DoneForDay`, `done_for_day`, `DONE FOR DAY` and a bridge's `DoneDay` are one spelling, `80DONEDAY`.
- `State::rank` on a value that does not open with two digits -> `None`, and every lifecycle predicate answers `false`.
- The width bounds the value read, never the spelling, so `TradeHasBeenReleasedToClearing` reads to the ten-byte `80TRDRELS`.
- No default value: a published vocabulary spells the members, so there is no neutral one, and an empty text cell entering the column is null ([Cast](../cast.md#empty-text)). `00UNKNOWN` is a stated value, not an absence.
- `StringEnum::STATES` is the listing - the thirty-five stored values, ordered first to last - reached by the logical name `state`.

## Commands

=== "Rust"

    ```bash
    cargo test --features "parquet iceberg" --manifest-path rust/Cargo.toml -p yggdryl --test types -- datatype::coded vocabulary::
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/types/test_datatype.py -k "registered_code"
    ```

=== "JavaScript"

    ```bash
    node --test --test-name-pattern="registered code" node/tests/types/datatype.test.js
    ```
