# Enums

A closed vocabulary stored as the unsigned integer code of its member - `uint8` where every code fits a byte, `uint16` where one passes 255: the member is the value, the code is what a column holds, and the codes are laid out so the stored integers sort the way the vocabulary is read.

An enum is not a [code](../codes/index.md). A code is an identity over a published registry that stores as the text it is; an enum's members are fixed by the crate - its own lifecycle, or a FIX code set it adopts whole - each already a fact a reader asks about, so it stores as the number that answers by order alone: a [state](state.md) by lifecycle, a [market data kind](marketdatakind.md) by its MsgCat value, a [market data type](marketdatatype.md) by the FIX code set it types, a [side](side.md) and a [time in force](timeinforce.md) in FIX's wire order. The family is the `enum` range of identifier bytes, `0xc0`-`0xcf`.

## Contract

| Aspect | Rule |
| --- | --- |
| Owns | Each enum's `DataType` variant, `Field` leaf and `Scalar` variant; the family is the `enum` range of `DataTypeId` bytes, not a type |
| Validates | At the value door, once: a member, the code of one, or a spelling one of its vocabularies names; anything else is refused naming the enum |
| Lazy | Nothing - the member tables are static |
| Cached | The Arrow projection of a [`Field`](../field.md), built once per field |
| Refuses | An integer that is the code of no member, a spelling that names none; `code_width`, `string_parameters` and `bytes_parameters`, which an enum has none of |
| Errors | Rust `Error::InvalidDataType { kind, reason }` where `kind` is the enum's own name; Python `ValueError`; JavaScript throws |
| Storage | Arrow `UInt8` (`side`, `marketdatakind`, `timeinforce`) or `UInt16` (`state`, `marketdatatype`) under the enum's own extension name - the width is the leaf's, so a column is one value buffer of one or two bytes a row and a row group's min and max are its first and last member |
| Identity | The extension *name*: `yggdryl.state` over `uint16` is a state, `yggdryl.side` over `uint8` a side, and the same integers under no name are the integers they are |
| Intake | Any integer column - signed or unsigned, any width, nested as deep as it likes - casts into an enum column, each value read as a code and refused by name where it names no member; a text column is read as spellings |
| Crossing | Rust holds the member; Python the member of an `enum.IntEnum` built from the core's table; JavaScript the member's name, beside a frozen object mapping every name to its code ([below](#enum-facts-in-the-bindings)) |
| Rust only | `EnumValue`, the contract every enum answers - `ALL`, `KIND`, `EXTENSION_NAME`, `type Repr` (`u8` or `u16`, the width a column stores), `code` (answering a `Repr`), `as_str`, `description`, `from_code`, `read`, `read_code`; `Scalar::is_enum`, `enum_code`, `enum_name` |

## Pages

| Page | Vocabulary | Members | Storage | Arrow extension |
| --- | --- | ---: | --- | --- |
| [State](state.md) | What state one thing is in, from asked for to ended, over FIX and a scheduler | 61 | `uint16` | `yggdryl.state` |
| [MarketDataKind](marketdatakind.md) | What kind of market data an element is: FIX's MsgCat code set | 26 | `uint8` | `yggdryl.marketdatakind` |
| [MarketDataType](marketdatatype.md) | What type of its kind a market element is: FIX's `OrdType(40)`, `QuoteType(537)`, `TrdType(828)`, `MDEntryType(269)`, `TradeReportType(856)`, `QuoteRequestType(303)`, `MassCancelRequestType(530)` and `SubscriptionRequestType(263)` values | 118 | `uint16` | `yggdryl.marketdatatype` |
| [Side](side.md) | Which side of the market a trade took: FIX `Side(54)` | 18 | `uint8` | `yggdryl.side` |
| [TimeInForce](timeinforce.md) | How long an order stands: FIX `TimeInForce(59)` | 15 | `uint8` | `yggdryl.timeinforce` |

## A spelling reads by its words

Every enum reads a spelling its exact vocabularies - the stored name, a wire code, a standard's name folded - name nothing by the words it is made of. `Part-Filled`, `partial fill`, `filled partially`, `ORDER FILLED` and `partfilled` are one set of words once each word is read as the word it abbreviates or inflects (`part`, `partial` and `partially` are one word; so are `cxl` and `cancelled`, `rej` and `rejected`, `pend` and `pending`), a run of letters is cut into known words, and the words that say nothing (`order`, `status`, `fully`) are dropped; that set is the one `PARTIALLY_FILLED`'s own names make. One reading or none: a set two members make names neither, and a word no name uses leaves the spelling unread, so the value door still refuses it by name. A spelling read this way is kept - at most 1,024 per enum, because a column's distinct spellings are few and text from outside must not grow the process - so a column repeating it reads it once, allocation-free.

=== "Rust"

    ```rust
    use yggdryl::{DataType, Scalar, State};

    assert_eq!(State::from_spelling("order fill"), Some(State::Filled));
    assert_eq!(State::from_spelling("Part-Filled"), Some(State::PartiallyFilled));
    assert_eq!(State::from_spelling("pending cxl"), Some(State::PendingCancel));
    assert_eq!(DataType::State.scalar("partial fill order")?, Scalar::State(State::PartiallyFilled));
    assert_eq!(State::from_spelling("partially frobnicated"), None);
    ```

=== "Python"

    ```python
    from yggdryl import DataType, State

    assert State.from_spelling("order fill") is State.FILLED
    assert State.from_spelling("Part-Filled") is State.PARTIALLY_FILLED
    assert DataType("state").scalar("pending cxl").as_py() is State.PENDING_CANCEL
    assert State.from_spelling("partially frobnicated") is None
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType } = require('yggdryl')

    const state = new DataType('state')
    assert.equal(state.scalar('order fill').asJs(), 'FILLED')
    assert.equal(state.scalar('Part-Filled').asJs(), 'PARTIALLY_FILLED')
    assert.throws(() => state.scalar('partially frobnicated'), /partially frobnicated/)
    ```

## Enum facts in the bindings

One rule for every enum value a binding answers, wherever it comes from - a `Scalar`, a column read back, or a getter answering an enum fact: `state`, `side`, `marketdatakind` and `marketdatatype` on every [market data](../../graph/market-data.md) leaf, `timeinforce` on every [operation](../../graph/operation.md), and a [FIX message](../../fix/message.md)'s `msgcat`, `marketdatatype` and `timeinforce`. Python answers the member of the enum's `IntEnum` - `yggdryl.State`, `yggdryl.MarketDataKind`, `yggdryl.MarketDataType`, `yggdryl.Side`, `yggdryl.TimeInForce` - which is the code it stores and renders as its name. JavaScript answers the member's name, and the frozen object of the same name maps it to the code a column stores. Either binding takes a member, its code or any spelling the enum reads wherever it takes one.

=== "Rust"

    ```rust
    use yggdryl::graph::{Element, Event, Market, MarketKind, OrderEvent};
    use yggdryl::{MarketDataKind, Side, State};

    let mut order = OrderEvent::at(1_700_000_000_000_000_000);
    order.set_crosscode("O-1001".to_owned());
    order.set_side(Side::Buy, true);
    order.set_state(State::New);
    order.finalize();

    // Rust holds the member itself.
    assert_eq!(order.get_side(), Side::Buy);
    assert_eq!(*order.get_state(), State::New);
    assert_eq!(MarketKind::OrderEvent.marketdatakind(), MarketDataKind::Order);
    assert_eq!(Side::Buy.code(), 1);
    ```

=== "Python"

    ```python
    from yggdryl import MarketDataKind, Side, State, graph

    order = graph.OrderEvent(
        1_700_000_000_000_000_000, crosscode="O-1001", side="BUYS", state="NEW"
    )

    # Python answers the member of the enum's `IntEnum`.
    assert order.side is Side.BUYS
    assert order.state is State.NEW
    assert order.marketdatakind is MarketDataKind.ORDR
    # A member is the code it stores and reads as its name.
    assert order.side == 1 and f"{order.state}" == "NEW"
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { MarketDataKind, Side, State, graph } = require('yggdryl')

    const order = new graph.OrderEvent(1_700_000_000_000_000_000n, {
      crosscode: 'O-1001',
      side: 'BUYS',
      state: 'NEW',
    })

    // JavaScript answers the member's name; the frozen object maps it to its code.
    assert.equal(order.side, 'BUYS')
    assert.equal(order.state, 'NEW')
    assert.equal(order.marketdatakind, 'ORDR')
    assert.equal(Side[order.side], 1)
    assert.equal(State[order.state], 2001)
    assert.equal(MarketDataKind[order.marketdatakind], 10)
    ```

## Edges

- A member of one enum is refused by another's value door, even where its code is one of the other's: a `state`, a `side` and a `marketdatakind` of one code are three values, and their digests differ.
- The default value of every enum is its code `0` member - `UNKN` for a side, a kind, a type and a time in force, `UNKNOWN` for a state - a stated value rather than an absence; an empty text cell entering an enum column is null ([Cast](../cast.md#empty-text)).
- JSON, TOML, YAML and XML write a member as its stored name; the [value stream](../value-stream.md) and a digest feed its four-byte little-endian code under the enum's own identifier whatever width the column stores, so a digest does not move with the storage; Iceberg stores it as an `int`.
- A cast between two enums reads every member again, and refuses a code the target names nothing by: a `state` column's `2001` is no `marketdatakind`.

## Commands

=== "Rust"

    ```bash
    cargo test --manifest-path rust/Cargo.toml -p yggdryl --test root -- state:: marketdatakind:: marketdatatype:: side:: timeinforce:: datatype_id::
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/test_state.py python/tests/test_marketdatakind.py python/tests/test_marketdatatype.py python/tests/test_side.py python/tests/test_timeinforce.py -q
    ```

=== "JavaScript"

    ```bash
    node --test node/tests/state.test.js node/tests/marketdatakind.test.js node/tests/marketdatatype.test.js node/tests/side.test.js node/tests/timeinforce.test.js
    ```
