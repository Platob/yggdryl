# yggdryl-fix in Rust

Everything is at the crate root (`yggdryl::{FixCodec, FixRegistry, FixMsg, fix_schema, ...}`),
no feature flag needed; the market traits (`get_side`, `get_crosscode`) need
`use yggdryl::graph::{Element, Event, Market}`. The dictionary path below is the
`config/fix` folder of a yggdryl checkout - point it at your own copy.

## Load the committed dictionary once and share it

`FixRegistry::from_handle` loads and resolves the whole catalog; wrap it in one
`Arc` and hand clones to every codec.

```rust
use std::sync::Arc;

use yggdryl::local::LocalFolder;
use yggdryl::{FixCodec, FixRegistry};

// `config/fix` of a yggdryl checkout: the dictionary is not shipped in the crate.
let dictionary = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../config/fix");
let registry = Arc::new(FixRegistry::from_handle(&LocalFolder::new(dictionary)?)?);
assert_eq!(registry.msgtype("D")?.name(), "newordersingle");
assert_eq!(registry.msgtype("8")?.name(), "executionreport");

// One catalog, many codecs: every codec holds the same `Arc`.
let orders = FixCodec::new(Arc::clone(&registry)).with_include_msgtypes(["D"]);
let reports = FixCodec::new(Arc::clone(&registry)).with_include_msgtypes(["8"]);
assert!(Arc::ptr_eq(orders.registry(), reports.registry()));

// A folder holding no catalog loads the crate's own definitions and creates nothing.
let empty = std::env::temp_dir().join(format!("ygg-skill-fix-none-{}", std::process::id()));
assert_eq!(FixRegistry::from_handle(&LocalFolder::new(&empty)?)?.len(), FixRegistry::new().len());
assert!(!empty.exists());
```

`FixRegistry::global()` answers the process default (an installed registry,
then `YGGDRYL_FIX_REGISTRY`, then `~/.config/fix`, then the empty registry),
resolved once; `FixRegistry::install_global(registry)` must run before anything
resolves it. `FixMsg::new` links the default; everything else takes the
registry you pass.

## Look fields up in the one namespace

A bare integer is a tag, a string a folded name or a path, and an identity only
ever travels as `FixKey::Id` / `field_by_id`.

```rust
use yggdryl::local::LocalFolder;
use yggdryl::{DataType, FieldPath, FixId, FixKey, FixRegistry};

let dictionary = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../config/fix");
let registry = FixRegistry::from_handle(&LocalFolder::new(dictionary)?)?;

assert_eq!(registry.field(55)?.name(), "symbol");
assert_eq!(registry.field_by_name("Sym_Bol")?.as_fix().tag()?, Some(55));
assert_eq!(registry.field_by_tag(453)?.dtype(), &DataType::Int32);
// The counter names the group it opens; a path reaches through the group.
assert_eq!(registry.field_by_counter(453)?.name(), "parties");
assert_eq!(registry.field_by_path(&FieldPath::from_str("Parties.PartyID")?)?.as_fix().tag()?, Some(448));

// The identity is the tag and the folded name; a bare integer is never one.
let id = registry.field(55)?.as_fix().id()?.expect("a tagged field");
assert_eq!(id, FixId::of(55, "Symbol")?);
assert_eq!(registry.field(FixKey::Id(id))?.name(), "symbol");
assert!(registry.get_field(id.digest()).is_none());

// A field reads its values by a named code set the dictionary holds once.
let side = registry.field(54)?;
let codes = registry.codeset_of(side).expect("Side reads by a code set");
assert_eq!(codes.name(), "sidecodeset");
assert_eq!(codes.code_name("1"), Some("Buy"));
assert_eq!(codes.code_value("Sell"), Some("2"));
```

## Put FIX facts on a field

The `FIX:` vocabulary is metadata on an ordinary `Field`, read with `as_fix()`
and written atomically with `as_fix_mut()`.

```rust
use yggdryl::{DataType, FixId};

let mut field = DataType::decimal128(20, 8)?.nullable_field("OrderQty");
field.as_fix_mut().set_tag(38)?;
field.as_fix_mut().set_names(["Qty", "Quantity"])?;
field.as_fix_mut().set_branches(["Venue", "desk"])?;

assert_eq!(field.as_fix().tag()?, Some(38));
assert_eq!(field.get_metadata("FIX:names"), Some("[\"Qty\",\"Quantity\"]"));
assert_eq!(field.as_fix().branches().collect::<Vec<_>>(), ["desk", "venue"]);
// Derived on every read, never stored; a folded rename keeps it.
assert_eq!(field.as_fix().id()?, Some(FixId::of(38, "order_qty")?));
assert!(!field.has_metadata("FIX:id"));

// A refusal names the key and leaves the field unchanged.
let error = field.as_fix_mut().set_tag(0).unwrap_err();
assert!(error.to_string().contains("FIX:tag"), "{error}");
assert_eq!(field.as_fix().tag()?, Some(38));
```

## Decode one captured line

`parse_line` takes a whole captured line - verb, prose and remarks included - and
answers a lazy iterator of every message it carries.

```rust
use std::sync::Arc;

use yggdryl::local::LocalFolder;
use yggdryl::{FieldPath, FixCodec, FixRegistry, Scalar};

let dictionary = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../config/fix");
let codec = FixCodec::new(Arc::new(FixRegistry::from_handle(&LocalFolder::new(dictionary)?)?));

let mut messages = codec.parse_line(b"recv 8=FIX.4.4|35=D|453=1|448=BROKER|452=1|10=000|")?;
let message = messages.next().expect("one frame")?;
assert!(messages.next().is_none());
assert_eq!(message.by_tag(453)?, Scalar::from(1_i32));
assert_eq!(message.by_path(&FieldPath::from_str("Parties[0].PartyID")?)?.as_str(), Some("BROKER"));

// Two frames on one line are two messages; a sentence is none.
let both = b"8=FIX.4.4|35=D|11=A|10=001|8=FIX.4.4|35=8|37=O1|10=002|";
assert_eq!(codec.parse_line(both)?.count(), 2);
assert!(codec.parse_line(b"After Enrichment -> ACCOUNT=A1 SIDE=1")?.next().is_none());
// The single-frame door refuses a body holding a second frame.
assert!(codec.parse_fix_line(both).is_err());
```

## Read only the message types you need

The type filter is read off the `35=` a row states, before a message is built,
so a refused keepalive costs one look.

```rust
use std::sync::Arc;

use yggdryl::local::LocalFolder;
use yggdryl::{DEFAULT_REFUSED_MSGTYPES, FixCodec, FixRegistry};

let dictionary = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../config/fix");
let registry = Arc::new(FixRegistry::from_handle(&LocalFolder::new(dictionary)?)?);
let lines = [
    "8=FIX.4.4|35=0|112=TEST|10=0|",
    "8=FIX.4.4|35=D|11=A|55=AAPL|10=0|",
    "8=FIX.4.4|35=8|37=O1|10=0|",
];

// The default refuses Heartbeat, TestRequest and the untyped row.
let codec = FixCodec::new(Arc::clone(&registry));
assert_eq!(codec.exclude_msgtypes(), DEFAULT_REFUSED_MSGTYPES);
assert_eq!(codec.parse_lines(lines).count(), 2);

// Naming what to read, in any spelling the dictionary resolves, is the whole answer.
let orders = FixCodec::new(Arc::clone(&registry)).with_include_msgtypes(["NewOrderSingle"]);
assert_eq!(orders.include_msgtypes(), ["D"]);
assert_eq!(orders.parse_lines(lines).count(), 1);

// An empty refusal reads the session whole, as an audit does.
let audit = FixCodec::new(registry).with_exclude_msgtypes::<[&str; 0], &str>([]);
assert_eq!(audit.parse_lines(lines).count(), 3);

// Routing before any parse: the stated type, read without a dictionary.
assert_eq!(FixCodec::infer_msgtype_bytes(b"8=FIX.4.4|35=AE|"), Some(b"AE".as_slice()));
```

## Read typed facts off a message

A message holds each fact once: the header on `header()`, the market reading on
the graph traits, everything else in the content row the dictionary typed.

```rust
use std::sync::Arc;

use yggdryl::graph::{Element, Event, Market};
use yggdryl::local::LocalFolder;
use yggdryl::{Decimal, FixCodec, FixRegistry, Scalar};

let dictionary = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../config/fix");
let codec = FixCodec::new(Arc::new(FixRegistry::from_handle(&LocalFolder::new(dictionary)?)?));
let message = codec.parse_fix_line(b"8=FIX.4.4|35=D|52=20260102-10:15:30|11=A1|55=AAPL|54=1|38=100|44=10.5|10=0|")?;

assert_eq!((message.header().beginstring(), message.header().msgtype()), ("FIX.4.4", "D"));
// A coded value reads as its name; the wire keeps its code.
assert_eq!(message.by_tag(54)?.as_str(), Some("BUY"));
assert_eq!(message.get_side().as_str(), "BUY");
assert_eq!(message.get_quantity(), Some(Decimal::from_int(100)));
assert_eq!(message.by_name("symbol")?, Scalar::from("AAPL"));
// The first stated OrderID, ClOrdID, ... names the order's chain.
assert_eq!(message.get_crosscode(), "A1");
// Instants are i64 nanoseconds since the epoch, UTC.
assert_eq!(message.get_currunix(), 1_767_348_930_000_000_000);
// The entries are the content row as a tree; the lifted 11, 38 and 44 are not in it.
let names: Vec<&str> = message.entries().iter().map(yggdryl::FixEntry::name).collect();
assert_eq!(names, ["symbol", "side", "timeinforce"]);
```

## Compose a message and write facts

`FixMsg::with_registry` builds a message from a root `Field` and its value;
`set` and `remove` write a typed fact's holder or the row, and settle the
identity again.

```rust
use std::sync::Arc;

use yggdryl::graph::Element;
use yggdryl::{DataType, Field, FixMsg, FixRegistry, Scalar, StructType};

let tagged = |name: &str, tag: i32| -> yggdryl::Result<Field> {
    let mut field = DataType::utf8().nullable_field(name);
    field.as_fix_mut().set_tag(tag)?;
    Ok(field)
};
let (msgtype, clordid, symbol) = (tagged("MsgType", 35)?, tagged("ClOrdID", 11)?, tagged("Symbol", 55)?);
let registry = Arc::new(FixRegistry::from_fields([msgtype.clone(), clordid.clone(), symbol.clone()])?);

let root = DataType::from(StructType::from_fields([msgtype, clordid, symbol])?).required_field("NewOrderSingle");
let value = Scalar::from_struct([
    ("MsgType", Scalar::from("D")),
    ("ClOrdID", Scalar::from("A1")),
    ("Symbol", Scalar::from("AAPL")),
])?;
let mut message = FixMsg::with_registry(Arc::clone(&registry), root, value)?;
assert_eq!(message.header().msgtype(), "D");
assert_eq!(message.get_crosscode(), "A1");

let before = message.get_currhashcode();
message.set("Symbol", Scalar::from("MSFT"))?;
assert_eq!(message.by_tag(55)?, Scalar::from("MSFT"));
assert_ne!(message.get_currhashcode(), before, "a write settles the identity again");
assert_eq!(message.remove(55)?, Some(Scalar::from("MSFT")));
assert!(message.get_by_tag(55).is_none());
```

## Encode a message back to the wire

`into_text` / `into_bytes` re-emit what the message states - header, lifted
fields, entries, trailer - with the separator you choose; `digest` hashes those
bytes whatever the separator.

```rust
use std::sync::Arc;

use yggdryl::local::LocalFolder;
use yggdryl::{FixCodec, FixRegistry, SOH};

let dictionary = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../config/fix");
let codec = FixCodec::new(Arc::new(FixRegistry::from_handle(&LocalFolder::new(dictionary)?)?));
let message = codec.parse_fix_line(b"8=FIX.4.4|35=D|52=20260102-10:15:30.000|55=AAPL|54=1|38=100|10=000|")?;

// Header, lifted quantity, entries (the side as its wire code, the derived
// day order), then the trailer.
let text = message.into_text('|')?;
assert_eq!(text, "8=FIX.4.4|35=D|52=20260102-10:15:30|38=100|55=AAPL|54=1|59=0|10=000|");
assert_eq!(message.into_bytes(SOH), text.replace('|', "\u{1}").into_bytes());

// Emission is idempotent, and the digest ignores the separator.
let again = codec.parse_fix_line(text.as_bytes())?;
assert_eq!(again.into_text('|')?, text);
assert_eq!(again.digest(), message.digest());
```

## Stream a capture into Arrow rows

`parse_text_arrow_reader` takes the batches a text reader answers (a payload
column, `body` by default) and answers FIX rows, one per message, the capture's
own columns leading; parsing is pooled across `threads`.

```rust
use std::sync::Arc;

use yggdryl::local::LocalFolder;
use yggdryl::{DataType, FixCodec, FixRegistry, Scalar, Serie, StructType};

let dictionary = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../config/fix");
let registry = Arc::new(FixRegistry::from_handle(&LocalFolder::new(dictionary)?)?);

let capture = DataType::from(StructType::from_fields([
    DataType::utf8().required_field("url"),
    DataType::Int64.required_field("rownum"),
    DataType::utf8().required_field("body"),
])?)
.required_field("line");
let rows = [
    Scalar::from_sequence([Scalar::from("file:///s.log"), Scalar::from(7_i64), Scalar::from("recv 8=FIX.4.4|35=D|11=A|55=AAPL|10=0|")]),
    Scalar::from_sequence([Scalar::from("file:///s.log"), Scalar::from(8_i64), Scalar::from("heartbeat emitted seq=7")]),
];
let batch = Serie::from_scalars(capture, rows)?.into_arrow_batch()?;
let source = yggdryl::arrow::batch_reader(batch.schema(), [batch]);

let read = FixCodec::new(registry)
    .with_threads(4)
    .with_batch_row_size(10_000)
    .parse_text_arrow_reader(source)?;
// The schema is decided before a row is read: the capture leads, `fixentries` closes.
let schema = read.schema();
assert_eq!(schema.field(0).name(), "url");
assert_eq!(schema.fields().last().map(|field| field.name().as_str()), Some("fixentries"));
// One row per message: the sentence carried none.
let rows: usize = read.map(|batch| batch.map(|batch| batch.num_rows())).sum::<Result<_, _>>()?;
assert_eq!(rows, 1);
```

## Read a log file with a row header

Let the text reader cut lines and capture the row header, then hand its batches
to the codec: an `mtime` capture dates the line, and every other capture either
fills the FIX field it is named after or leads the row as context.

```rust
use std::sync::Arc;

use arrow_array::Array as _;
use yggdryl::holder::Buffer;
use yggdryl::local::LocalFolder;
use yggdryl::media::IORecordOptions as _;
use yggdryl::graph::Event;
use yggdryl::text::TextOptions;
use yggdryl::{FixCodec, FixMsg, FixRegistry, IOBase as _, IOMedia as _, Timezone, Url};

let dictionary = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../config/fix");
let registry = Arc::new(FixRegistry::from_handle(&LocalFolder::new(dictionary)?)?);

let log = b"2026-01-02 10:15:30.250 IN  8=FIX.4.4|35=D|11=A1|55=AAPL|54=1|38=100|10=0|\n\
2026-01-02 10:15:30.500 OUT 8=FIX.4.4|35=8|11=A1|37=O1|150=0|39=0|55=AAPL|10=0|\n\
2026-01-02 10:15:31.000 OUT 8=FIX.4.4|35=0|10=0|\n";
let options = TextOptions::new()
    .try_with_rowheader(r"^(?P<mtime>\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2}\.\d{3}) (?P<level>IN|OUT) +")?
    .with_timezone(Timezone::UTC);
let handle = Buffer::from_bytes(log.to_vec())
    .with_media_type(Url::from_str("file:///session.log")?.media_type())
    .into_text_with(options);
let lines = handle.read_arrow_reader(&handle.record_options()?)?;

let batches = FixCodec::new(Arc::clone(&registry))
    .with_threads(2)
    .parse_text_arrow_reader(lines)?
    .collect::<Result<Vec<_>, _>>()?;
let batch = &batches[0];
assert_eq!(batch.num_rows(), 2, "the heartbeat is refused by default");
assert!(batch.schema().index_of("level").is_ok(), "an unnamed capture leads the row");
// The line's clock became each message's instant; no SendingTime was invented.
assert_eq!(batch.column_by_name("currunix").expect("currunix").null_count(), 0);
assert_eq!(batch.column_by_name("sendingtime").expect("sendingtime").null_count(), 2);

// Line by line: tell the codec what the captures are called, once.
let codec = FixCodec::new(registry).with_capture_names(["mtime", "level"]);
let messages: Vec<FixMsg> = codec.parse_text_lines(handle.read_text_lines()?).collect::<yggdryl::Result<_>>()?;
let recorded: Vec<Option<i64>> = messages.iter().map(Event::get_recdunix).collect();
assert_eq!(recorded, [Some(1_767_348_930_250_000_000), Some(1_767_348_930_500_000_000)]);
```

## Land messages in the fixed row and back

`fix_schema` is the one row every message answers as; `arrow_reader` and
`messages` cross between messages and batches, and `write_arrow_reader` writes
batches back as wire lines.

```rust
use std::sync::Arc;

use yggdryl::local::LocalFolder;
use yggdryl::{FixCodec, FixMsg, FixRegistry, fix_column_of, fix_schema};

let dictionary = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../config/fix");
let registry = Arc::new(FixRegistry::from_handle(&LocalFolder::new(dictionary)?)?);
let codec = FixCodec::new(Arc::clone(&registry)).with_separator(b'|');
let schema = fix_schema(&registry, "fix")?;
// Columns are the dictionary's folded names; the tag is on each column.
assert_eq!(schema.index_of("msgtype"), fix_column_of(&schema, 35));

let lines = ["8=FIX.4.4|35=D|11=ORDER-1|55=AAPL|54=1|9999=x|10=0|", "8=FIX.4.4|35=8|17=E1|37=O9|31=12.75|32=50|10=0|"];
let parsed: Vec<FixMsg> = codec.parse_lines(lines).collect::<yggdryl::Result<_>>()?;

// One message as one row, and back.
let row = parsed[0].into_row(&schema)?;
assert_eq!(FixMsg::from_row(Arc::clone(&registry), &schema, &row)?.into_row(&schema)?, row);

// A stream of messages as batches, and the batches as messages again.
let again: Vec<FixMsg> = codec
    .messages(codec.arrow_reader(schema.clone(), parsed.clone())?)
    .collect::<yggdryl::Result<_>>()?;
assert_eq!(again.len(), 2);

// And out to the wire, one line per row.
let mut sink = Vec::new();
assert_eq!(codec.write_arrow_reader(codec.arrow_reader(schema, again)?, &mut sink)?, 2);
assert!(String::from_utf8(sink)?.starts_with("8=FIX.4.4|35=D|11=ORDER-1|9999=x|"));
```

## Chain an order's lifecycle

`lifecycle` is the one cross-message stage: it collects the finite capture,
sorts it, folds repeated deliveries and chains each message to the live one of
its order under one `crossuuid`.

```rust
use std::sync::Arc;

use yggdryl::graph::{Element, Event};
use yggdryl::local::LocalFolder;
use yggdryl::{FixCodec, FixMsg, FixRegistry, fix_schema};

let dictionary = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../config/fix");
let registry = Arc::new(FixRegistry::from_handle(&LocalFolder::new(dictionary)?)?);
let codec = FixCodec::new(Arc::clone(&registry));
let lines = [
    "8=FIX.4.4|35=8|37=O1|150=F|39=2|14=100|151=0|31=10.5|32=100|55=AAPL|52=20260102-10:15:33.100|10=0|",
    "8=FIX.4.4|35=D|11=A1|55=AAPL|54=1|38=100|44=10.5|52=20260102-10:15:30.250|10=0|",
    "8=FIX.4.4|35=8|11=A1|37=O1|150=0|39=0|55=AAPL|52=20260102-10:15:30.500|10=0|",
];
// Parsed, nothing follows anything: each names only the chain it spells.
let parsed: Vec<FixMsg> = codec.parse_lines(lines).collect::<yggdryl::Result<_>>()?;
assert!(parsed.iter().all(|held| held.get_seqnum() == 0 && held.get_prevuuid().is_none()));

let [order, ack, fill] = codec.lifecycle(parsed).collect::<yggdryl::Result<Vec<_>>>()?.try_into().expect("three");
// Sorted by event time, joined by the identifiers each message went by.
assert_eq!((order.get_seqnum(), ack.get_seqnum(), fill.get_seqnum()), (0, 1, 2));
assert_eq!(ack.get_prevuuid(), Some(order.get_curruuid()));
assert_eq!(fill.get_prevuuid(), Some(ack.get_curruuid()));
assert!([&ack, &fill].iter().all(|held| held.get_crossuuid() == order.get_crossuuid()));
assert_eq!(fill.get_state().as_str(), "80FILLED");

// Rows already in Arrow chain in place, under the schema they were read with.
let schema = fix_schema(&registry, "fix")?;
let rows = codec.arrow_reader(schema, codec.parse_lines(lines))?;
let chained: usize = codec.lifecycle_arrow_reader(rows)?.map(|batch| batch.map(|batch| batch.num_rows())).sum::<Result<_, _>>()?;
assert_eq!(chained, 3);
```

## Turn FIX into market operations and books

`market_operations` admits what a book folds, expands each message into graph
leaves and sorts them by the instant a book folds them at; `BookIterator` then
walks them. Compose `lifecycle` in front when predecessor state matters.
`market_arrow_reader` writes the sorted operations as `marketdata` rows, and
`market_operations_arrow_reader` is its twin over batches of FIX rows already
in Arrow.

```rust
use std::sync::Arc;

use yggdryl::graph::{BookIterator, MarketData, MarketKind};
use yggdryl::local::LocalFolder;
use yggdryl::{FixCodec, FixMsg, FixRegistry, fix_schema};

let dictionary = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../config/fix");
let registry = Arc::new(FixRegistry::from_handle(&LocalFolder::new(dictionary)?)?);
let codec = FixCodec::new(registry.clone());
// The update arrives before the snapshot it follows.
let lines = [
    "8=FIX.4.4|35=X|52=20260921-10:00:01|55=AAPL|268=2|279=1|269=0|278=B1|270=101|271=11|279=0|269=2|278=T1|270=101|271=2|10=0|",
    "8=FIX.4.4|35=W|52=20260921-10:00:00|55=AAPL|268=2|269=0|278=B1|270=100|271=10|269=1|278=A1|270=102|271=12|10=0|",
];
let capture: Vec<FixMsg> = codec.parse_lines(lines).collect::<yggdryl::Result<_>>()?;

let operations: Vec<MarketData> = codec.market_operations(codec.lifecycle(capture.clone())).collect::<yggdryl::Result<_>>()?;
assert_eq!(operations.last().map(MarketData::kind), Some(MarketKind::ExecutionEvent));
let books = BookIterator::new(operations.into_iter(), 0, false)?.collect::<yggdryl::Result<Vec<_>>>()?;
assert_eq!(books.len(), 2);
assert_eq!(books[1].bid().best_price().map(|price| price.to_string()).as_deref(), Some("101"));

// The book door is strict: the same capture out of order is refused.
assert!(codec.book_arrow_reader(capture.clone(), 0, false)?.any(|batch| batch.is_err()));
// The sorted operations as `marketdata` rows.
let rows: usize = codec.market_arrow_reader(capture.clone())?.map(|batch| batch.map(|batch| batch.num_rows())).sum::<Result<_, _>>()?;
assert_eq!(rows, 4);
// The same operations off the capture's FIX rows.
let fixed = codec.arrow_reader(fix_schema(&registry, "fix")?, capture)?;
let rows: usize = codec.market_operations_arrow_reader(fixed)?.map(|batch| batch.map(|batch| batch.num_rows())).sum::<Result<_, _>>()?;
assert_eq!(rows, 4);
```

## Build and commit a dictionary

Build fields, components, groups and code sets in memory - a set before the
field naming it - then `commit` writes the shard tree and `from_handle` reads it
back whole.

```rust
use yggdryl::local::LocalFolder;
use yggdryl::{DataType, FieldPath, FixCode, FixRegistry, IOBase, StructType};

let path = std::env::temp_dir().join(format!("ygg-skill-fix-store-{}", std::process::id()));
let mut root = LocalFolder::new(&path)?;

let mut count = DataType::Int32.nullable_field("NoPartyIDs");
count.as_fix_mut().set_tag(453)?;
let mut party_id = DataType::utf8().nullable_field("PartyID");
party_id.as_fix_mut().set_tag(448)?;
let mut registry = FixRegistry::from_fields([count, party_id])?;

// A Struct files as a component, a Serie of one as a group.
let mut member = registry.field(448)?.clone();
member.as_fix_mut().set_field_ref("PartyID")?;
let party = DataType::from(StructType::from_fields([member])?).required_field("Party");
registry.insert(party.clone())?;
let mut parties = DataType::serie(party).nullable_field("Parties");
parties.as_fix_mut().set_counter(453)?;
parties.as_fix_mut().set_component("Party")?;
registry.insert(parties)?;

// The vocabulary first, then the field that reads by it.
registry.set_codeset("sidecodeset", &[FixCode::new("Buy", "1"), FixCode::new("Sell", "2")])?;
let mut side = DataType::utf8().nullable_field("Side");
side.as_fix_mut().set_tag(54)?;
side.as_fix_mut().set_codeset("sidecodeset")?;
registry.insert(side)?;

let report = registry.commit(&mut root)?;
assert!(!report.written.is_empty() && report.removed.is_empty());
assert!(path.join("codesets/sidecodeset.json").is_file());
let reloaded = FixRegistry::from_handle(&root)?;
assert_eq!(reloaded, registry);
assert_eq!(reloaded.field_by_path(&FieldPath::from_str("Parties.PartyID")?)?.as_fix().tag()?, Some(448));
root.remove(true)?;
```

## Fold a venue CBlock into a dictionary

`FixRegistry::from_cfb_file` reads one Ullink CBlock (`.cfb`) into a registry
and its declared roots, stamping the dialect on everything it produced;
`add_cfb_file` / `add_cfb_files` fold one or a glob of them into a held
registry (dialect defaulting to each file's stem), and `merge_with` folds a
whole other registry. Every fold is atomic.

```rust
use yggdryl::FixRegistry;
use yggdryl::local::{LocalFile, LocalFolder};

let path = std::env::temp_dir().join(format!("ygg-skill-fix-cfb-{}", std::process::id()));
std::fs::create_dir_all(&path)?;
let cblock = |name: &str| format!(r#"<?xml version="1.0" encoding="US-ASCII"?>
<cplugin-configuration fix-version="4.4">
  <vocabulary><vocabulary-tag name="4" alt="AdvSide" type="char" /></vocabulary>
  <maps><map name="ADVSIDE"><entries><entry key="{name}" value="B" /></entries></map></maps>
</cplugin-configuration>
"#);
std::fs::write(path.join("alpha.cfb"), cblock("buy"))?;
std::fs::write(path.join("beta.cfb"), cblock("venue_buy"))?;

let (venue, roots) = FixRegistry::from_cfb_file(&LocalFile::new(path.join("alpha.cfb"))?, Some("venue"))?;
assert_eq!(venue.field(4)?.as_fix().branches().collect::<Vec<_>>(), ["venue"]);
assert!(roots.is_empty());

let mut registry = FixRegistry::new();
let (files, _added, _merged) = registry.add_cfb_files(&LocalFolder::new(&path)?, "*.cfb", None)?;
assert_eq!(files, 2);
// Ascending URL order, each file stamped with its stem.
assert_eq!(registry.field(4)?.as_fix().branches().collect::<Vec<_>>(), ["alpha", "beta"]);

registry.merge_with(&venue)?;
assert_eq!(registry.dialects(), ["alpha", "beta", "venue"]);
std::fs::remove_dir_all(&path)?;
```

## Gotchas in Rust

- `FixCodec::new` takes `Arc<FixRegistry>`; clone the `Arc`, never the registry.
- Every stream door yields `Result` items: `collect::<yggdryl::Result<Vec<_>>>()`
  stops at the first refused line; iterate and match to skip one and go on.
- The graph getters (`get_crosscode`, `get_side`, `get_currunix`) are trait
  methods: import `yggdryl::graph::{Element, Event, Market}`.
- `with_exclude_msgtypes([])` needs its types spelled:
  `with_exclude_msgtypes::<[&str; 0], &str>([])`.
- `FixDedup` (drop an adjacent republication) and `registry.with_default_aliases()`
  are Rust-only.
- `FixRegistry::install_global` fails once the default is resolved; install at
  startup, before any `FixRegistry::global()` or `FixMsg::new`.
