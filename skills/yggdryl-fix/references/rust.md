# yggdryl-fix in Rust

Everything is at the `yggdryl-fix` crate's root
(`yggdryl_fix::{FixCodec, FixRegistry, FixMsg, fix_schema, ...}`), no feature
flag needed; the graph getters are trait methods - `get_crosscode` and
`get_transunix` need `use yggdryl::graph::{Element, Event}`, `get_side`
`use yggdryl_market::graph::Market`. The dictionary path below is the
`config/fix` folder of a yggdryl checkout - point it at your own copy.

## Load the committed dictionary once and share it

`FixRegistry::from_handle` loads and resolves the whole catalog; wrap it in one
`Arc` and hand clones to every codec.

```rust
use std::sync::Arc;

use yggdryl::local::LocalFolder;
use yggdryl_fix::{FixCodec, FixRegistry};
yggdryl_fix::install()?;

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

`FixRegistry::from_env()` answers the process default (an installed registry,
then `YGGDRYL_FIX_REGISTRY`, then `~/.config/fix`, then the empty registry),
resolved once; `FixRegistry::install_env(registry)` must run before anything
resolves it. `FixMsg::new` links the default; everything else takes the
registry you pass.

## Look fields up in the one namespace

A bare integer is a tag, a string a folded name or a path, and an identity only
ever travels as `FixKey::Id` / `field_by_id`.

```rust
use yggdryl::local::LocalFolder;
use yggdryl::{DataType, FieldPath};
use yggdryl_fix::{FixField, FixId, FixKey, FixRegistry};
yggdryl_fix::install()?;

let dictionary = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../config/fix");
let registry = FixRegistry::from_handle(&LocalFolder::new(dictionary)?)?;

assert_eq!(registry.field(55)?.name(), "symbol");
assert_eq!(FixField::new(registry.field_by_name("Sym_Bol")?).tag()?, Some(55));
assert_eq!(registry.field_by_tag(453)?.dtype(), &DataType::Int32);
// The counter names the group it opens; a path reaches through the group.
assert_eq!(registry.field_by_counter(453)?.name(), "parties");
assert_eq!(FixField::new(registry.field_by_path(&FieldPath::from_str("Parties.PartyID")?)?).tag()?, Some(448));
// A name reads four word pairs either way: offer/ask, size/qty, bid/demand, px/price.
assert_eq!(FixField::new(registry.field_by_name("AskPrice")?).tag()?, Some(133));
assert_eq!(FixField::new(registry.field_by_name("DemandQty")?).tag()?, Some(134));

// The identity is the tag and the folded name; a bare integer is never one.
let id = FixField::new(registry.field(55)?).id()?.expect("a tagged field");
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

The `FIX:` vocabulary is metadata on an ordinary `Field`, read through
`FixField::new(&field)` and written atomically through
`FixFieldMut::new(&mut field)`.

```rust
use yggdryl::DataType;
use yggdryl_fix::{FixField, FixFieldMut, FixId};
yggdryl_fix::install()?;

let mut field = DataType::decimal128(20, 8)?.nullable_field("OrderQty");
FixFieldMut::new(&mut field).set_tag(38)?;
FixFieldMut::new(&mut field).set_names(["Qty", "Quantity"])?;
FixFieldMut::new(&mut field).set_sources(["Venue", "desk"])?;

assert_eq!(FixField::new(&field).tag()?, Some(38));
assert_eq!(field.get_metadata("FIX:names"), Some("[\"Qty\",\"Quantity\"]"));
assert_eq!(FixField::new(&field).sources().collect::<Vec<_>>(), ["desk", "venue"]);
assert_eq!(field.get_metadata("FIX:sources"), Some(r#"["desk","venue"]"#));
// Derived on every read, never stored; a folded rename keeps it.
assert_eq!(FixField::new(&field).id()?, Some(FixId::of(38, "order_qty")?));
assert!(!field.has_metadata("FIX:id"));

// A refusal names the key and leaves the field unchanged.
let error = FixFieldMut::new(&mut field).set_tag(0).unwrap_err();
assert!(error.to_string().contains("FIX:tag"), "{error}");
assert_eq!(FixField::new(&field).tag()?, Some(38));
```

## Decode one captured line

`parse_line` takes a whole captured line - verb, prose and remarks included - and
answers a lazy iterator of every message it carries.

```rust
use std::sync::Arc;

use yggdryl::local::LocalFolder;
use yggdryl::{FieldPath, Scalar};
use yggdryl_fix::{FixCodec, FixRegistry};
yggdryl_fix::install()?;

let dictionary = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../config/fix");
let codec = FixCodec::new(Arc::new(FixRegistry::from_handle(&LocalFolder::new(dictionary)?)?));

let mut messages = codec.parse_line(b"recv 8=FIX.4.4|35=D|453=1|448=BROKER|452=1|10=000|")?;
let message = messages.next().expect("one frame")?;
assert!(messages.next().is_none());
// The group is its list: its length is the count, and tag 453 reaches nothing.
assert_eq!(message.by_name("parties")?.as_sequence().map(<[Scalar]>::len), Some(1));
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
use yggdryl_fix::{DEFAULT_REFUSED_MSGTYPES, FixCodec, FixRegistry};
yggdryl_fix::install()?;

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

use yggdryl::graph::{Element, Event};
use yggdryl_market::graph::Market;
use yggdryl::local::LocalFolder;
use yggdryl::{Decimal, Scalar};
use yggdryl_fix::{FixCodec, FixRegistry};
use yggdryl_market::MarketDataKind;
yggdryl_fix::install()?;

let dictionary = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../config/fix");
let codec = FixCodec::new(Arc::new(FixRegistry::from_handle(&LocalFolder::new(dictionary)?)?));
let message = codec.parse_fix_line(b"8=FIX.4.4|35=D|52=20260102-10:15:30|11=A1|55=AAPL|54=1|38=100|44=10.5|202=105|10=0|")?;

assert_eq!((message.header().beginstring(), message.header().msgtype()), ("FIX.4.4", "D"));
// The category its type files under, and the option strike it identifies.
assert_eq!(message.marketdatakind(), MarketDataKind::Order);
assert_eq!(message.get_strikepx(), Some(Decimal::from_int(105)));
// A coded value reads as its name; the wire keeps its code.
assert_eq!(message.by_tag(54)?.as_str(), Some("BUYS"));
assert_eq!(message.get_side().as_str(), "BUYS");
assert_eq!(message.get_quantity(), Some(Decimal::from_int(100)));
assert_eq!(message.by_name("symbol")?, Scalar::from("AAPL"));
// The first stated OrderID, ClOrdID, ... names the order's chain, stored under its side.
assert_eq!(message.get_crosscode(), "10:1:A1");
// Instants are i64 nanoseconds since the epoch, UTC.
assert_eq!(message.get_transunix(), 1_767_348_930_000_000_000);
// The entries are the content row as a tree; the lifted 11, 38 and 44 are not in it.
let names: Vec<&str> = message.entries().iter().map(yggdryl_fix::FixEntry::name).collect();
assert_eq!(names, ["symbol", "side", "strikeprice", "timeinforce"]);
```

## Compose a message and write facts

`FixMsg::with_registry` builds a message from a root `Field` and its value;
`set` and `remove` write a typed fact's holder or the row, and settle the
identity again.

```rust
use std::sync::Arc;

use yggdryl::graph::Element;
use yggdryl::{DataType, Field, Scalar, StructType};
use yggdryl_fix::{FixFieldMut, FixMsg, FixRegistry};
yggdryl_fix::install()?;

let tagged = |name: &str, tag: i32| -> yggdryl::Result<Field> {
    let mut field = DataType::utf8().nullable_field(name);
    FixFieldMut::new(&mut field).set_tag(tag)?;
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
assert_eq!(message.get_crosscode(), "10:0:A1");

let before = message.get_hashcode();
message.set("Symbol", Scalar::from("MSFT"))?;
assert_eq!(message.by_tag(55)?, Scalar::from("MSFT"));
assert_ne!(message.get_hashcode(), before, "a write settles the identity again");
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
use yggdryl_fix::{FixCodec, FixRegistry, SOH};
yggdryl_fix::install()?;

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
own columns following the shared ones; parsing is pooled across `threads`.

```rust
use std::sync::Arc;

use yggdryl::local::LocalFolder;
use yggdryl::{DataType, Scalar, Serie, StructType};
use yggdryl_fix::{FixCodec, FixRegistry};
yggdryl_fix::install()?;

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
// The schema is decided before a row is read: the shared columns lead, the capture follows them, `fixentries` closes.
let schema = read.schema();
assert_eq!(schema.field(0).name(), "uuid");
let at = schema.index_of("url")?;
assert_eq!(schema.field(at - 1).name(), "partyids");
assert_eq!(schema.field(at + 2).name(), "body");
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
use yggdryl_fix::{FixCodec, FixMsg, FixRegistry};
use yggdryl::{IOBase as _, IOMedia as _, Timezone, Url};
yggdryl_fix::install()?;

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
assert!(batch.schema().index_of("level").is_ok(), "an unnamed capture is a column of the row");
// The line's clock became each message's instant; no SendingTime was invented.
assert_eq!(batch.column_by_name("transunix").expect("transunix").null_count(), 0);
assert_eq!(batch.column_by_name("sendingtime").expect("sendingtime").null_count(), 2);

// Line by line: tell the codec what the captures are called, once.
let codec = FixCodec::new(registry).with_capture_names(["mtime", "level"]);
let messages: Vec<FixMsg> = codec.parse_text_lines(handle.read_text_lines()?).collect::<yggdryl::Result<_>>()?;
let sent: Vec<Option<i64>> = messages.iter().map(Event::get_sendunix).collect();
assert_eq!(sent, [Some(1_767_348_930_250_000_000), Some(1_767_348_930_500_000_000)]);
```

## Land messages in the fixed row and back

`fix_schema` is the one row every message answers as; `arrow_reader` and
`messages` cross between messages and batches, and `write_arrow_reader` writes
batches back as wire lines. A key no dictionary resolves lands in `metadata`,
unless it names an identifier the message captures - then it rides
`fixentries` under `0:<key>`.

```rust
use std::sync::Arc;

use yggdryl_market::graph::Market;
use yggdryl::local::LocalFolder;
use yggdryl_fix::{FixCodec, FixMsg, FixRegistry, fix_column_of, fix_schema};
use yggdryl_market::IdType;
use yggdryl::Scalar;
yggdryl_fix::install()?;

let dictionary = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../config/fix");
let registry = Arc::new(FixRegistry::from_handle(&LocalFolder::new(dictionary)?)?);
let codec = FixCodec::new(Arc::clone(&registry)).with_separator(b'|');
let schema = fix_schema(&registry, "fix")?;
// Columns are the dictionary's folded names; the tag is on each column.
assert_eq!(schema.index_of("msgtype"), fix_column_of(&schema, 35));

let lines = ["8=FIX.4.4|35=D|11=ORDER-1|55=AAPL|54=1|18=G|9999=x|10=0|", "8=FIX.4.4|35=8|17=E1|37=O9|31=12.75|32=50|10=0|"];
let parsed: Vec<FixMsg> = codec.parse_lines(lines).collect::<yggdryl::Result<_>>()?;

// One message as one row, and back.
let row = parsed[0].into_row(&schema)?;
// What no column holds is `fixentries`, keyed `tag:name`; a key no dictionary
// resolves is no field, and lands in `metadata` under its own spelling.
let cell = |name: &str| row.get(schema.index_of(name).expect("a fixed column")).expect("a cell");
assert_eq!(cell("fixentries").get_key_str("18:execinst").and_then(Scalar::as_str), Some("G"));
assert_eq!(cell("metadata").get_key_str("9999").and_then(Scalar::as_str), Some("x"));
assert_eq!(FixMsg::from_row(Arc::clone(&registry), &schema, &row)?.into_row(&schema)?, row);

// An unresolved key naming an identifier is captured: held in its set, it rides
// `fixentries` under `0:<key>` and leaves `metadata`.
let bridged = codec.parse_fix_line(b"8=FIX.4.4|35=D|11=ORDER-2|55=AAPL|54=1|RICCODE=AAPL.O|10=0|")?;
assert_eq!(bridged.get_securityids().get(&IdType::Ric), Some("AAPL.O"));
let captured = bridged.into_row(&schema)?;
let captured_cell = |name: &str| captured.get(schema.index_of(name).expect("a fixed column")).expect("a cell");
assert_eq!(captured_cell("fixentries").get_key_str("0:riccode").and_then(Scalar::as_str), Some("AAPL.O"));
assert!(captured_cell("metadata").get_key_str("riccode").is_none());

// A stream of messages as batches, and the batches as messages again.
let again: Vec<FixMsg> = codec
    .messages(codec.arrow_reader(schema.clone(), parsed.clone())?)
    .collect::<yggdryl::Result<_>>()?;
assert_eq!(again.len(), 2);

// And out to the wire, one line per row.
let mut sink = Vec::new();
assert_eq!(codec.write_arrow_reader(codec.arrow_reader(schema, again)?, &mut sink)?, 2);
assert!(String::from_utf8(sink)?.starts_with("8=FIX.4.4|35=D|11=ORDER-1|18=G|9999=x|"));
```

## Chain an order's lifecycle

`lifecycle` is the one cross-message stage: it collects the finite capture,
sorts it, folds repeated deliveries and chains each message to the live one of
its order and side under one `crossuuid`, within one market data kind (`marketdatakind`); a
report stating no side joins the one side alive under its identifiers - a
message joins the live one of its kind, side and instrument sharing one identifier
of the same type and value, every type of its `identifiers` but a shared one
(`trdmatchid`, `quotereqid`, `mdreqid`, a parent slot), and the first value a lineage
field names - and every message of a chain carries the chain's first
`crosscode`, a replace under a new `ClOrdID` included. A message citing two
live chains is joined to neither: it stands under its own identity and carries
a `FixAnomaly` under `crosscode` naming both, warned once per kind. A fill's
execution, split off at the parse, is a chain of its own and never restates,
follows or ends its order. A codec pinned `with_sorted_lifecycle(true)` reads a source already in
instant order as it comes, one epoch hour at a time, and answers the same walk. The walk yields
each `uuid` once within `dedup_window_ms` of event time, one minute unless
the codec says otherwise; `with_dedup_window_ms(0)` yields every restated twin too.
A snapshot grid's view is the live message as of its tick: dated at it, so its
`uuid` is that instant's, with the live message's content and place.
An order's chain counts its fills once each by `ExecID(17)` - or
`SecondaryExecID(527)` - over its first stated `CumQty(14)`: `cumqty` and
`leavesqty` are the count's, a fill delivered again, a status reply (`150=I`,
`17=0`) and a leg's report (`442=2`) count nothing, a bust (`150=H` with
`ExecRefID(19)`) takes the fill it names back, a correction (`150=G`)
replaces it, and a partial fill whose count reaches the order quantity reads
`FILLED`; a stated total that disagrees is warned, never adopted, and an ended
chain's fills are remembered for the window, so a late copy starts no chain
([lifecycle](https://platob.github.io/yggdryl/fix/lifecycle/#an-orders-fills-are-counted-once)).

```rust
use std::sync::Arc;

use yggdryl::graph::{Element, Event};
use yggdryl_market::graph::Market;
use yggdryl::local::LocalFolder;
use yggdryl_fix::{FixCodec, FixMsg, FixRegistry, fix_schema};
use yggdryl_market::{MarketDataKind, Side};
use yggdryl::State;
yggdryl_fix::install()?;

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
assert!(parsed.iter().all(|held| held.get_prevuuid().is_none()));
// Each takes its place at its instant: the execution stands after its report.
let places: Vec<u64> = parsed.iter().map(Event::get_seqnum).collect();
assert_eq!(places, [0, 1, 0, 0]);

// Three lines, four messages: the fill's report and the execution it reports.
assert_eq!(parsed.len(), 4);

let [order, ack, fill, execution] = codec.lifecycle(parsed).collect::<yggdryl::Result<Vec<_>>>()?.try_into().expect("four");
// Sorted by event time, joined by the identifiers each message went by; each
// follows one of an earlier instant, so each keeps its own place.
assert_eq!((order.get_seqnum(), ack.get_seqnum(), fill.get_seqnum()), (0, 0, 0));
assert_eq!(ack.get_prevuuid(), Some(order.get_uuid()));
assert_eq!(fill.get_prevuuid(), Some(ack.get_uuid()));
assert!([&ack, &fill].iter().all(|held| held.get_crossuuid() == order.get_crossuuid()));
// The reports stated no side: they joined the buy alive under A1 and O1.
assert!([&ack, &fill].iter().all(|held| held.get_side() == Side::Buy && held.get_crosscode() == "10:1:A1"));
assert_eq!((fill.marketdatakind(), *fill.get_state()), (MarketDataKind::Order, State::Filled));
// Every walked message states when its chain began.
assert!([&ack, &fill].iter().all(|held| held.get_creaunix() == Some(order.get_transunix())));
assert_eq!((execution.marketdatakind(), *execution.get_state()), (MarketDataKind::Execution, State::Filled));
assert_eq!((execution.get_seqnum(), execution.get_prevuuid()), (1, None));

// Rows already in Arrow chain in place, under the schema they were read with.
let schema = fix_schema(&registry, "fix")?;
let rows = codec.arrow_reader(schema, codec.parse_lines(lines))?;
let chained: usize = codec.lifecycle_arrow_reader(rows)?.map(|batch| batch.map(|batch| batch.num_rows())).sum::<Result<_, _>>()?;
assert_eq!(chained, 4);
```

## Share what lifecycles learn about instruments

A lifecycle learns each message's instrument into an `Instruments`, keyed by
its cross code - a real ISIN for a security, `class:body` for an FX pair or a
derivative, its `QY` number minted - with its CFI code, country, market,
ticker, currency, pair and security codes, and fills what later messages of
that instrument leave unsaid, as `derived` identifiers, the ticker, CFI and
currency facts and the instrument's cross code as `instcode`, never the wire;
a parse through the same codec fills derived identifiers from the table its
door fixed. A codec without one learns into a collection of each walk's own;
`with_instruments` shares one across walks run one after another, bound to a
store with `from_url` and written back with `commit` only where it moved;
`Instruments::from_env` lays its store over the embedded common instruments
`Instruments::seeded()` holds. An instrument carries the national number its
ISIN embeds - Holcim's Valor below - and each listing its market's country's
currency where it states none. A structured product's EUSIPA category is
learned off a bridge's own key (`EUSIPACode`, `OMS_SSPACategory`, ...) as the
instrument's `eusipacode`, an `Eusipa`; the key is lifted into no identifier
map. A row's `instcode` joins the instruments table on `crosscode`.

```rust
use std::sync::{Arc, Mutex};

use yggdryl_market::graph::Market;
use yggdryl::local::LocalFolder;
use yggdryl_market::{Eusipa, Instrument, Instruments};
use yggdryl_fix::{FixCodec, FixMsg, FixRegistry};
yggdryl_fix::install()?;

let dictionary = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../config/fix");
let registry = Arc::new(FixRegistry::from_handle(&LocalFolder::new(dictionary)?)?);
let instruments = Arc::new(Mutex::new(Instruments::new()));
let codec = FixCodec::new(registry).with_instruments(Arc::clone(&instruments));

// The first walk states Holcim's ISIN, RIC, CFI code, ticker and market.
let stated = ["8=FIX.4.4|35=D|11=A|22=4|48=CH0012214059|454=1|455=HOLN.S|456=5|461=ESVUFR|55=HOLN|207=XSWX|10=0|"];
let parsed: Vec<FixMsg> = codec.parse_lines(stated).collect::<yggdryl::Result<_>>()?;
codec.lifecycle(parsed).collect::<yggdryl::Result<Vec<_>>>()?;
assert_eq!(instruments.lock().unwrap().get("CH0012214059").and_then(|row| row.get(&yggdryl_market::IdType::Ric)), Some("HOLN.S"));
assert_eq!(instruments.lock().unwrap().get("CH0012214059").and_then(|row| row.get(&yggdryl_market::IdType::Valor)), Some("1221405"));

// A bridge key states a structured product's category beside its ISIN.
let product = ["8=FIX.4.4|35=D|11=C|22=4|48=CH0123456789|55=ACMEL|207=XSWX|OMS_SSPACategory=2300|10=0|"];
let parsed: Vec<FixMsg> = codec.parse_lines(product).collect::<yggdryl::Result<_>>()?;
codec.lifecycle(parsed).collect::<yggdryl::Result<Vec<_>>>()?;
let category = instruments.lock().unwrap().get("CH0123456789").and_then(Instrument::eusipacode);
assert_eq!(category, Some(Eusipa::new(2300)?));
assert_eq!(category.and_then(|code| code.name()), Some("Constant Leverage Certificate"));

// A later parse naming only the ticker on the market takes the ISIN from
// the table, derived; the walk fills the CFI code as a market fact.
let later = ["8=FIX.4.4|35=D|11=B|55=HOLN|207=XSWX|10=0|"];
let parsed: Vec<FixMsg> = codec.parse_lines(later).collect::<yggdryl::Result<_>>()?;
assert_eq!(parsed[0].get_isincode(), Some("CH0012214059"));
let walked = codec.lifecycle(parsed).collect::<yggdryl::Result<Vec<_>>>()?;
assert_eq!(walked[0].get_cficode().map(|code| code.as_str()), Some("ESVUFR"));
assert_eq!(walked[0].get_instcode(), Some("CH0012214059"), "the instrument's cross code");

// An FX pair no agency numbers: its code and its minted number are spelled
// from the message alone, and the walk learns its instrument.
let pair = ["8=FIX.4.4|35=D|11=F|55=EUR/USD|54=1|38=1000000|10=0|"];
let parsed: Vec<FixMsg> = codec.parse_lines(pair).collect::<yggdryl::Result<_>>()?;
assert_eq!((parsed[0].get_instcode(), parsed[0].get_isincode()), (Some("IF:EUR/USD"), Some("QYLTVIRYHNX5")));
codec.lifecycle(parsed).collect::<yggdryl::Result<Vec<_>>>()?;
assert_eq!(Instrument::minted_number("IF:EUR/USD").as_str(), "QYLTVIRYHNX5");
assert!(instruments.lock().unwrap().get("IF:EUR/USD").is_some());
```

## Follow a replace chain's parents

A message that states a chain identity again under another value is a step in
its chain: `lifecycle` keeps the value before it as the type's parent
(`orderid` leaves `parentorderid` and the chain's first as `origorderid`,
`clordid` leaves `origclordid`), and joins a replace to its order by the first value its lineage field
names (`OrigClOrdID(41)`, `OrigTradeID(1126)`, `TradeReportRefID(572)`), under
the base; only a chain identity has parents, so an `ExecID(17)` carries none.
`registry.parents_of(&base)` lists them, nearest first, from the
`FIX:parents` a field states.

```rust
use std::sync::Arc;

use yggdryl::graph::Element;
use yggdryl_market::graph::Operation;
use yggdryl::local::LocalFolder;
use yggdryl_fix::{FixCodec, FixMsg, FixRegistry};
use yggdryl_market::IdType;
yggdryl_fix::install()?;

let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../config/fix");
let reader = FixCodec::new(Arc::new(FixRegistry::from_handle(&LocalFolder::new(root)?)?));
let lines: [&[u8]; 4] = [
    b"8=FIX.4.4|35=D|11=C1|55=AAPL|54=1|38=10|44=100|52=20260921-10:00:00|10=0|",
    b"8=FIX.4.4|35=8|11=C1|37=O1|150=0|39=0|55=AAPL|52=20260921-10:00:01|10=0|",
    b"8=FIX.4.4|35=G|11=C2|41=C1|37=O2|55=AAPL|54=1|38=10|44=101|52=20260921-10:00:02|10=0|",
    b"8=FIX.4.4|35=G|11=C3|41=C2|37=O3|55=AAPL|54=1|38=10|44=102|52=20260921-10:00:03|10=0|",
];
let parsed: Vec<FixMsg> = reader.parse_lines(lines).collect::<yggdryl::Result<_>>()?;
let chained: Vec<FixMsg> = reader.lifecycle(parsed).collect::<yggdryl::Result<_>>()?;

// What each message holds under each type, `-` where it holds none.
let held = |message: &FixMsg| -> [String; 5] {
    ["orderid", "parentorderid", "origorderid", "clordid", "origclordid"].map(|kind| {
        let kind: IdType = kind.parse().expect("a type");
        message.get_identifiers().get(&kind).unwrap_or("-").to_owned()
    })
};
assert_eq!(held(&chained[0]), ["-", "-", "-", "C1", "-"]);
assert_eq!(held(&chained[1]), ["O1", "-", "-", "C1", "-"]);
// Each replace names the value before it and the chain's first.
assert_eq!(held(&chained[2]), ["O2", "O1", "O1", "C2", "C1"]);
assert_eq!(held(&chained[3]), ["O3", "O2", "O1", "C3", "C2"]);
// One chain: the replaces joined the order by the parent they state.
assert!(chained.iter().all(|message| message.get_crossuuid() == chained[0].get_crossuuid()));
```

## Split fills and batches at the parse

The parse splits what a message reports, once, so nothing downstream states a
fill twice: an execution report is its order's report (`marketdatakind` `ORDR`, its
own state; `QUOT` where it names a `QuoteID(117)`) - one of no fill from its
parse - and one that fills adds one `EXEC` message reading `FILLED`, chained
under its `ExecID(17)` as given, else `TradeID=<TradeID(1003)>`; a trade
(`AE`) adds one sided execution per `NoSides(552)` occurrence; a batch (`marketdatakind` `ORDB`, `QUOB`, `EXEB` or `TRDB`:
an order list, a mass order, a cross, a mass quote, a match report) adds one
message per entry, filed under its item (`ORDR`, `QUOT`, `EXEC`, `TRAD`) - a
mass quote's entry one quote holding both its legs - chained by the order the
entry names and split again as its category is. Each split message names its
source in `srcuuids`. A quote is never split: a bid and an offer are the two
legs of one message, stored under side `0`. An acknowledgement of an execution
(`BN`, `Q`) states no fact of its order, so it answers no market leaf.

```rust
use std::sync::Arc;

use yggdryl::graph::{Element, Event};
use yggdryl_market::graph::Market;
use yggdryl::local::LocalFolder;
use yggdryl_fix::{FixCodec, FixMsg, FixRegistry};
use yggdryl_market::{MarketDataKind, Side};
use yggdryl::State;
yggdryl_fix::install()?;

let dictionary = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../config/fix");
let codec = FixCodec::new(Arc::new(FixRegistry::from_handle(&LocalFolder::new(dictionary)?)?));

let fill = b"8=FIX.4.4|35=8|52=20260921-10:00:00|17=E-1|37=O-9|11=C-9|39=1|150=F|55=AAPL|54=1|38=100|14=40|32=40|31=10.5|10=0|";
let [report, execution]: [FixMsg; 2] = codec.parse_line(fill)?.collect::<yggdryl::Result<Vec<_>>>()?.try_into().expect("two");
assert_eq!((report.marketdatakind(), *report.get_state()), (MarketDataKind::Order, State::PartiallyFilled));
assert_eq!((execution.marketdatakind(), *execution.get_state()), (MarketDataKind::Execution, State::Filled));
assert!(execution.get_srcuuids().contains(&report.get_uuid()));
// An order or an execution message stores its cross code under its side; the fill is a chain of its own.
assert_eq!((report.get_crosscode(), execution.get_crosscode()), ("10:1:O-9", "8:1:E-1"));

let quote = b"8=FIX.4.4|35=S|52=20260921-10:00:00|117=Q1|55=AAPL|15=USD|132=99|134=7|133=101|135=8|10=0|";
let [quote]: [FixMsg; 1] = codec.parse_line(quote)?.collect::<yggdryl::Result<Vec<_>>>()?.try_into().expect("one");
assert_eq!((quote.marketdatakind(), quote.get_side(), quote.get_crosscode()), (MarketDataKind::Quotation, Side::Both, "14:0:Q1"));
// Both legs on the one message, each in its currency, tagged `BOTH`; neither is the quote's own price.
assert_eq!(quote.get_price(), None);
assert_eq!((quote.get_bidpx(), quote.get_askpx(), quote.get_askqty()), (Some("99".parse()?), Some("101".parse()?), Some("8".parse()?)));
assert_eq!(quote.get_askccy().map(|ccy| ccy.as_str()), Some("USD"));

// An acknowledgement of an execution states no fact of its order: no market leaf.
let ack = codec.parse_fix_line(b"8=FIX.4.4|35=BN|52=20260921-10:00:01|17=E-1|37=O-9|1036=2|10=0|")?;
assert!(ack.market_data()?.is_empty());
```

## Turn FIX into market data and books

`market_data` admits orders, quotes, executions and `W`/`X` book messages - a
trade as the executions its parse split off - reads each as its one graph
leaf (a book message one per entry) and sorts them by the instant a book folds
them at; `BookIterator` then walks them, pruning the executions.
`book_arrow_reader` folds the same messages into book rows, one book per book
key, and takes a `Filter` over the `marketdata` row to narrow what folds. Compose `lifecycle` in front when predecessor state matters.
`market_arrow_reader` writes the sorted leaves as `marketdata` rows, and
`market_data_arrow_reader` is its twin over batches of FIX rows already in
Arrow.

```rust
use std::sync::Arc;

use yggdryl_market::graph::{BookIterator, Market, MarketData, MarketKind};
use yggdryl::local::LocalFolder;
use yggdryl_fix::{FixCodec, FixMsg, FixRegistry, fix_schema};
use yggdryl_market::{MarketDataKind, Side};
yggdryl_fix::install()?;

let dictionary = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../config/fix");
let registry = Arc::new(FixRegistry::from_handle(&LocalFolder::new(dictionary)?)?);
let codec = FixCodec::new(registry.clone());
// The update arrives before the snapshot it follows.
let lines = [
    "8=FIX.4.4|35=X|52=20260921-10:00:01|55=AAPL|268=2|279=1|269=0|278=B1|270=101|271=11|279=0|269=2|278=T1|270=101|271=2|10=0|",
    "8=FIX.4.4|35=W|52=20260921-10:00:00|55=AAPL|268=2|269=0|278=B1|270=100|271=10|269=1|278=A1|270=102|271=12|10=0|",
];
let capture: Vec<FixMsg> = codec.parse_lines(lines).collect::<yggdryl::Result<_>>()?;
assert!(capture.iter().all(|message| message.marketdatakind() == MarketDataKind::Book));

let leaves: Vec<MarketData> = codec.market_data(codec.lifecycle(capture.clone())).collect::<yggdryl::Result<_>>()?;
assert_eq!(leaves.len(), 4, "one leaf per entry");
assert_eq!(leaves.last().map(MarketData::kind), Some(MarketKind::ExecutionEvent));
let books = BookIterator::new(leaves.into_iter(), 0)?.collect::<yggdryl::Result<Vec<_>>>()?;
assert_eq!(books.len(), 2);
assert_eq!(books[1].best_price(Side::Buy).map(|price| price.to_string()).as_deref(), Some("101"));

// The book door does not sort: the same capture out of order is no error - the
// snapshot dated before the book it would fold into is left out, with a warning.
assert!(codec.book_arrow_reader(capture.clone(), 0, None)?.all(|batch| batch.is_ok()));
// The sorted leaves as `marketdata` rows.
let rows: usize = codec.market_arrow_reader(capture.clone())?.map(|batch| batch.map(|batch| batch.num_rows())).sum::<Result<_, _>>()?;
assert_eq!(rows, 4);
// The same leaves off the capture's FIX rows.
let fixed = codec.arrow_reader(fix_schema(&registry, "fix")?, capture)?;
let rows: usize = codec.market_data_arrow_reader(fixed)?.map(|batch| batch.map(|batch| batch.num_rows())).sum::<Result<_, _>>()?;
assert_eq!(rows, 4);
```

## Build and commit a dictionary

Build fields, components, groups and code sets in memory - a set before the
field naming it - then `commit` writes the shard tree and `from_handle` reads it
back whole.

```rust
use yggdryl::local::LocalFolder;
use yggdryl::{DataType, FieldPath, IOBase, StructType};
use yggdryl_fix::{FixCode, FixField, FixFieldMut, FixRegistry};
yggdryl_fix::install()?;

let path = std::env::temp_dir().join(format!("ygg-skill-fix-store-{}", std::process::id()));
let mut root = LocalFolder::new(&path)?;

// The counter is a field of the dictionary; no component or message lists it
// beside the group, whose length is its count.
let mut count = DataType::Int32.nullable_field("NoPartyIDs");
FixFieldMut::new(&mut count).set_tag(453)?;
let mut party_id = DataType::utf8().nullable_field("PartyID");
FixFieldMut::new(&mut party_id).set_tag(448)?;
let mut registry = FixRegistry::from_fields([count, party_id])?;

// A Struct files as a component, a Serie of one as a group.
let mut member = registry.field(448)?.clone();
FixFieldMut::new(&mut member).set_field_ref("PartyID")?;
let party = DataType::from(StructType::from_fields([member])?).required_field("Party");
registry.insert(party.clone())?;
let mut parties = DataType::serie(party).nullable_field("Parties");
FixFieldMut::new(&mut parties).set_counter(453)?;
FixFieldMut::new(&mut parties).set_component("Party")?;
registry.insert(parties)?;

// The vocabulary first, then the field that reads by it.
registry.set_codeset("sidecodeset", &[FixCode::new("Buy", "1"), FixCode::new("Sell", "2")])?;
let mut side = DataType::utf8().nullable_field("Side");
FixFieldMut::new(&mut side).set_tag(54)?;
FixFieldMut::new(&mut side).set_codeset("sidecodeset")?;
registry.insert(side)?;

let report = registry.commit(&mut root)?;
assert!(!report.written.is_empty() && report.removed.is_empty());
assert!(path.join("codesets/sidecodeset.json").is_file());
let reloaded = FixRegistry::from_handle(&root)?;
assert_eq!(reloaded, registry);
assert_eq!(FixField::new(reloaded.field_by_path(&FieldPath::from_str("Parties.PartyID")?)?).tag()?, Some(448));
root.remove(true)?;
```

## Fold a venue CBlock into a dictionary

`FixRegistry::from_cfb_file` reads one Ullink CBlock (`.cfb`) into a registry
and its declared roots, stamping the dialect in `FIX:sources` on everything it
produced and recording the dialect's entry in the registry's sources catalog -
the file's name and the `Side` its root's `type` names, read by `yggdryl_fix::plugin_side`;
`add_cfb_file` folds one into a held registry, `add_cfb_files` folds what the
locations it is handed hold - a glob every file it matches, a folder the `.cfb`
files directly inside it, a file itself, a file reached twice folding once -
and `merge_with` folds a whole other registry, dialect defaulting to each
file's own stem. A fold keeps every declaration the dictionary already holds.
The answered `FixMerge` counts `sources`, `added` and `merged`, `restated`
among the merged - a field whose source stated another precision of the
stored datatype (a CBlock's `float` against `decimal128`, `string` against
`ccy`) and folded under it - and lists in `dropped` each contradiction passed
over rather than refusing the whole source. `add_cfb_files` folds each file as
one mutation: a file it cannot read, parse or fold is left out alone, named in
`failed` as a `FixFailure` (`source`, `reason`), while the rest fold;
`add_cfb_file` and `merge_with` refuse whole only a source that leaves nothing
to keep - malformed XML or JSON, a catalog that does not validate.
`is_clean()` is `dropped` and `failed` both empty. What the reader cannot keep
of a file is a `log` warning naming the line, the column, the element and what
the reader did instead.

```rust
use yggdryl::holder::Holder;
use yggdryl::local::LocalFile;
use yggdryl_fix::{FixField, FixRegistry, FixSource};
use yggdryl_market::Side;
yggdryl_fix::install()?;

let path = std::env::temp_dir().join(format!("ygg-skill-fix-cfb-{}", std::process::id()));
std::fs::create_dir_all(&path)?;
let cblock = |name: &str| format!(r#"<?xml version="1.0" encoding="US-ASCII"?>
<cplugin-configuration fix-version="4.4" type="com.ullink.SellSideFIXCPluginCBlock">
  <vocabulary><vocabulary-tag name="4" alt="AdvSide" type="char" /></vocabulary>
  <maps><map name="ADVSIDE"><entries><entry key="{name}" value="B" /></entries></map></maps>
</cplugin-configuration>
"#);
std::fs::write(path.join("alpha.cfb"), cblock("buy"))?;
std::fs::write(path.join("beta.cfb"), cblock("venue_buy"))?;
std::fs::write(path.join("broken.cfb"), "<cplugin-configuration><vocabulary>")?;

let (venue, roots) = FixRegistry::from_cfb_file(&LocalFile::new(path.join("alpha.cfb"))?, Some("venue"))?;
assert_eq!(FixField::new(venue.field(4)?).sources().collect::<Vec<_>>(), ["venue"]);
// The catalog records the source once: its file and its plugin's role.
let entry = venue.get_source("venue").expect("the dialect's entry");
assert_eq!((entry.file(), entry.pluginside()), (Some("alpha.cfb"), Side::Sell));
assert!(roots.is_empty());

// `add_cfb_files` takes the locations alone: a folder holds the `.cfb`
// files directly inside it, a glob (`path.join("*.cfb")`) what it matches.
let mut registry = FixRegistry::new();
let merge = registry.add_cfb_files(&[Holder::local(&path)?], None)?;
assert_eq!(merge.sources, 2);
assert_eq!(merge.restated, 0, "both files type tag 4 alike");
assert!(merge.dropped.is_empty(), "{:?}", merge.dropped);
// One file is one mutation: the broken one is left out, the others fold.
assert_eq!(merge.failed.len(), 1);
assert!(merge.failed[0].source.as_deref().is_some_and(|url| url.ends_with("broken.cfb")));
assert!(!merge.is_clean());
// Ascending URL order, each file stamped with its stem.
assert_eq!(FixField::new(registry.field(4)?).sources().collect::<Vec<_>>(), ["alpha", "beta"]);
assert_eq!(registry.sources().map(FixSource::id).collect::<Vec<_>>(), ["alpha", "beta"]);

registry.merge_with(&venue)?;
assert_eq!(registry.dialects(), ["alpha", "beta", "venue"]);
std::fs::remove_dir_all(&path)?;
```

## Gotchas in Rust

- `FixCodec::new` takes `Arc<FixRegistry>`; clone the `Arc`, never the registry.
- Every stream door yields `Result` items, and only a source failure is an
  `Err`: what a line states that cannot be read is defaulted or left out with
  a `log` warning, so `collect::<yggdryl::Result<Vec<_>>>()` stops at a failing
  source alone. Install a `log` backend (`env_logger`, say, or the core's own
  `yggdryl::logging::basic_config(BasicConfig::new())`, the terminal line on
  standard error) to see the warnings.
- The graph getters (`get_crosscode`, `get_side`, `get_transunix`) are trait
  methods: import `yggdryl::graph::{Element, Event}` and
  `yggdryl_market::graph::Market`.
- `with_exclude_msgtypes([])` needs its types spelled:
  `with_exclude_msgtypes::<[&str; 0], &str>([])`.
- `FixDedup` (drop an adjacent republication) is Rust-only, and so are
  `FxSymbol` and `FxTenor`, the FX symbol reading the parse runs.
- `FixRegistry::install_env` fails once the default is resolved; install at
  startup, before any `FixRegistry::from_env()` or `FixMsg::new`.
