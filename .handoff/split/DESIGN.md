# Splitting yggdryl into six crates: the design

The program's prompt is `.handoff/next/MARKET_SPLIT_PROMPT.md`; this file is
what each session decided, with the evidence and the alternative it refused,
and the ledger every later slice extends. The user's decisions (U1-U7) are
restated as taken and are not reopened here.

## State

| Fact | Value |
| --- | --- |
| Base | `main` at `2ae97567465e7d583366c972314eae064ffb776e` (the squash merge of PR #206), byte-identical to `d4a6a9245` |
| Program branch | `ccr-0fe6f9d0-ruymat`: the harness assigned this branch and refuses pushes to any other, so under D1's harness clause it is the program branch. Later sessions find it by its S0 commit, never by name |
| Branched from | `origin/main` at `2ae975674`; HEAD recorded before any edit: `2ae97567465e7d583366c972314eae064ffb776e` |
| Draft PR | #209, "Split yggdryl into six crates", draft, never marked ready or merged by a session |
| Landed before S0's pins | P0 `3d8bf84d9` (`PluginSide` deleted, D22), P1 `08ae4c6b7` (the medium holds its options, D23); each pushed alone and its CI read green before the next |
| Version | `0.1.21`, which `main` already released; bumped only at S9 on the user's go |
| Sandbox | Linux x86_64, 4 cores, 15 GiB, `cargo 1.97.0`, `rustc` stable plus `1.94.0`, Python 3.12.3 in `python/.venv`, Node 22.22.0 |

## The crate map

| crate | path | holds | depends on | lands |
| --- | --- | --- | --- | --- |
| `yggdryl` | `rust/` | the core; from S1 the registered-kind shape (`DataType::Market`, `Field::Market`, `Scalar::Market`, `Serie::Market(MarketSerie)`) and the one register; from S2 the media register; `State` and the event vocabulary stay (D4) | - | - |
| `yggdryl-market` | `rust/market` | (D25: the 17 codes, `code.rs`, `country/tables.rs` and `mic/tables.rs` stay in the core) `marketdatakind`, `marketdatatype`, `side`, `timeinforce`; `idkey`, `identifier`, `idtype`, `idsource`, `securityid`, `eusipa`, `limit`; the ISIN registry and its seed; `graph/` less `element.rs`, `column.rs`, `element_column.rs`; the CLI's `market` code | `yggdryl` | S4 |
| `yggdryl-fix` | `rust/fix` | all of `fix/` and `constants.rs`, `fix_category.rs`, the `FixField` view, `State`'s FIX tables, D10's moves; `scripts/generate_fix_dictionary.py`, `config/fix/`, the dump, the hash pin, the `fix` and `fix_allocations` benches, `docs/fix/`, `skills/yggdryl-fix`, the CLI's FIX code | `yggdryl`, `yggdryl-market` | S4 |
| `yggdryl-avro` | `rust/avro` | `avro/` whole, with its own `snap` (D16) | `yggdryl` | S6 |
| `yggdryl-parquet` | `rust/parquet` | `parquet/` | `yggdryl` | S6 |
| `yggdryl-iceberg` | `rust/iceberg` | `iceberg/`, and `s3tables/` behind its own `s3tables` feature (D15) | `yggdryl`, `yggdryl-avro`, `yggdryl-parquet` | S6 |
| `yggdryl-excel` | `rust/excel` | `excel/` whole - the Office Open XML workbook medium over the core's `zip/`, `holder/`, `arrow/` and `Serie` doors - with `ExcelOptions`, `Excel<H>`, `ExcelSerie`, `rust/tests/excel/`, `rust/tests/interop/excel.rs`, `rust/benchmarks/media/excel.rs`, `docs/media/excel.md`; the `xlsx` MIME and media types stay core routing vocabulary, as `MimeType::PARQUET` does (D10) | `yggdryl` | S6b |

Binding packages: Python `yggdryl` (the one native module, every crate linked) and `yggdryl-market` (pure Python at `python/market/`, import path `yggdryl.market`, FIX inside it); npm `yggdryl` and `yggdryl-market` (JavaScript only at `node/market/`); the unpublished `yggdryl-cli` links every crate. One workspace version covers every artifact.

## The user's decisions

| U | decision |
| --- | --- |
| U1 | the six crates as tabled; FIX in its own crate |
| U2 | the extension point is `Scalar::Market`, `Serie::Market(MarketSerie)`, `DataType::Market`, `Field::Market` |
| U3 | the media leave through one media extension point sharing D8's mechanism with the market one |
| U4 | Python and Node each get a `yggdryl.market` package; FIX ships inside it (D11 confirms); the existing packages keep Avro, Parquet and Iceberg |
| U5 | the split first (S1, S2, S3, S4, S5, S6), the adaptations after (S7, S8) |
| U6 | the planned expression series is finished inside this program, after the split |
| U7 | `RecordOptions` becomes `MediaOptions` after the split, one sweep, no alias |
| U8 | the `pluginside` datatype is deleted before S0 and a FIX plugin's role is a `Side` (P0, one commit, D22); 21 kinds leave with the market |
| U9 | the medium holds its `RecordOptions` and the media serie keeps no copy of them - the medium's role is to state or infer the right options with their defaults (P1, one commit before S0's pins, D23) |
| U10 | `excel/` leaves too: `yggdryl-excel`, a seventh crate through the media point (D24, S6b); the program's title keeps the prompt's spelling so Start here finds it |

## Baseline counts at HEAD (`2ae975674`)

Every count below was re-run at HEAD with the prompt's variables and the
`.handoff/` exclusion. Each matches the prompt's figure exactly; no finding.

In the commands below `\|` is the table's escape for `|`, so a command is
copied with that character unescaped.

| What | Command | Count |
| --- | --- | --- |
| `rust/src` lines | `git ls-files 'rust/src/*.rs' \| xargs cat \| wc -l` | 394,119 |
| the market area | `fix/` 50,537; `graph/` 23,535; `isin_registry.rs` 3,476 + `isin_registry/` 740 | as the prompt's "about" figures |
| kind names in what stays | `git grep -nE "\b($K)\b" -- rust/src $STAY ':(exclude).handoff' \| wc -l` | 1,352 lines, 27 files; largest `scalar.rs` 273, `serie.rs` 176, `datatype.rs` 139, `serde.rs` 132, `datatype_id.rs` 108, `serie/value.rs` 68, `default.rs` 61, `value/canonical.rs` 56, `string.rs` 47, `budget.rs` 34, `vocabulary.rs` 28, `cast.rs` 23, `yaml/mod.rs` 22, `text/mod.rs` 22, `parser.rs` 22, `media/inference.rs` 22, `field.rs` 22, `arrow/extension.rs` 22, `graph/mod.rs` 21, `enums.rs` 21, `serie/order.rs` 15, `value/mod.rs` 12, `merge.rs` 2, `txhash/value.rs` 1, `datatype_kind.rs` 1, `boolean.rs` 1, `ascii.rs` 1 |
| `code_scalars!`/`enum_scalars!` sites | `git grep -nE '\b(code_scalars\|enum_scalars)!' -- rust/src` | 23 lines, 10 files (`xxhash/scalar.rs` 4, `scalar.rs` 3, two each in `arithmetic.rs`, `expression/display.rs`, `json/wire.rs`, `toml/wire.rs`, `valuestream.rs`, `variant.rs`, `xml/wire.rs`, `yaml/mod.rs`) |
| `DataTypeId::<market>` lines | `git grep -nE "DataTypeId::($K)\b" -- rust/src rust/tests` | 243 (80 in `rust/src`, 163 in `rust/tests`); outside them 15: `.api-inventory.txt` 11, `docs/types/codes/ccy.md`, `docs/types/codes/country.md`, `docs/types/datatype.md`, `skills/yggdryl-types/SKILL.md` one each |
| every `DataTypeId::` line in `rust/src` | `git grep -n 'DataTypeId::' -- rust/src` | 487 lines, 31 files |
| helper calls in what stays | `git grep -nE '\b(is_code\|code_width\|enum_code\|enum_name)\b' -- rust/src $STAY` | 65 (93 across `rust/src`) |
| docs and skills naming a variant | `git grep -nE "(Scalar\|DataType\|Field\|DataTypeId\|Serie)::($K)\b" -- docs skills` | 375 lines, 29 files |
| `iceberg/` naming `crate::avro`/`crate::parquet` | `git grep -nE 'crate::(avro\|parquet)' -- rust/src/iceberg` | 27 |
| core comment and doc mentions (D14) | `git grep -nE "^\s*//.*($R)" -- rust/src $STAY` | 90 lines, 20 files |
| `RecordOptions` family (S7) | `git grep -ow <word> -- . ':(exclude).handoff'` | `RecordOptions` 1,302; `IORecordOptions` 288; `record_options` 894; `recordOptions` 110; 239 files |
| S8 names | `git grep -nF <name> -- . ':(exclude).handoff'` | 0 for each of `SerieKind`, `push_plan`, `SelectSerie`, `FilterSerie`, `OrderSerie`, `LimitSerie`, `JoinSerie`, `Source::Serie` |
| `graph::` in `fix/` | `git grep -l '\bgraph::' -- rust/src/fix` / `-n` | 10 files, 69 lines (the prompt's "about 152" was an orientation figure; this is the baseline) |
| `IsinRegistry`/`IsinTable` in `fix/codec.rs` | `grep -nE 'IsinRegistry\|IsinTable' rust/src/fix/codec.rs` | 17 lines at `2ae975674` and at HEAD (the prompt's "about 31" was orientation; the 15 first written here was a miscount the review caught) |
| market sections of `.api-inventory.txt` | `grep -cE '^### yggdryl::(fix\|graph\|isin_registry\|idkey\|identifier\|idtype\|idsource\|eusipa\|limit\|code\|country\|ccy\|mic\|cfi\|isin\|cusip\|sedol\|bbg\|figi\|ric\|forex\|unit\|lei\|bic\|elf\|dti\|fisn\|marketdatakind\|marketdatatype\|side\|timeinforce\|state\|securityid\|market)' .api-inventory.txt` | 76 by this regex (the prompt's 78 names no regex); 368 sections in all at `2ae975674`, 367 at HEAD (P0 retired `pluginside`'s) |
| test code under any `src/` | `git grep -nE '#\[(cfg\(test\)\|test)\]\|^\s*mod tests' -- rust/src python/src node/src cli/src` | 0 |
| D6's items | derived by S3 and S6 from a scratch `git mv` plus `cargo check -p <crate> --message-format=short`; the prompt's "about" figures are orientation only | - |
| the media sites S2 names | re-counted by S2 at its HEAD | - |

## Decisions

### D1, base, branch and PR

Decided. Base `2ae975674`. The harness assigned `ccr-0fe6f9d0-ruymat` and
refuses pushes elsewhere, so that branch is the program branch (the prompt's
`split/crates` name is not used; `Start here` finds the branch by its S0
commit). S0 pushes its commit there and opens the draft PR "Split yggdryl into
six crates" against `main`; it is never marked ready or merged by a session.
Each later session fetches, checks the branch out and merges `origin/main`
into it (never rebases), re-running the slice's sweep over conflicts and
recording the merge in the handoff. The version stays `0.1.21` until S9.

### D2, the market shape

Decided: a closed shape over registered static descriptors.

- `MarketDescriptor`, one `static` per kind in the kind's own root file (the
  name `MarketKind` is taken: `yggdryl::graph::MarketKind` is the leaf kind a
  `MarketData` stands under, kept by D9), holds: the
  `DataTypeId` byte, the canonical name and the Arrow extension name as
  `const` items (D3); its storage, `Storage::Text { width }` (a code: Arrow
  `Utf8`, the width the value rule holds a cell to), `Storage::Code8` or
  `Storage::Code16` (an enum: Arrow `UInt8`/`UInt16`); for an enum its member
  table (`&'static [(u16, &'static str, &'static str)]`: code, stored name,
  description); its value rank and its `Shape` position (D19); function
  pointers for reading text into the canonical stored text (`read: fn(&str)
  -> Result<Str>`, the one door `new` is), the canonical check (`is_canonical:
  fn(&str) -> bool`, `Forex`'s canonicalization included) and the rank
  (`rank: fn(&str) -> u8`, with `max_rank`), the enum's spelling reader
  (`from_spelling: fn(&str) -> Option<u16>`) and merge rule.
- `DataType::Market(MarketType)` where `MarketType(&'static MarketDescriptor)`;
  equality compares the byte, order reads the kind's datatype rank and hash
  its `Shape` position (D19: three per-kind numbers, none the byte).
- `Field::Market(FieldOf<MarketType>)`, one leaf through `field_leaves!`.
- `Scalar::Market(MarketScalar)` with `MarketScalar { kind: &'static
  MarketDescriptor, payload: MarketPayload }` and `MarketPayload = Text(Str) |
  Code(u16)` (a `*Value` name is a trait's, AGENTS Public vocabulary),
  both fields private: a value is built only through the descriptor's
  validating door (`MarketDescriptor::scalar(&self, text)`, `::member(&self, code)`),
  so no crate lays out an unchecked value.
- `Serie::Market(MarketSerie)` with `MarketSerie = Text(Arc<Utf8StringSerie>)
  | Code8(Arc<UInt8Serie>) | Code16(Arc<UInt16Serie>)`: the three storages
  today's 23 variants share (`serie.rs:539-595`); the leaf's `field` holds the
  `MarketType`, so the kind is read off the field, never stored twice.
- Sizes: `Scalar` stays 48 bytes and `Serie` 40; S0's spike compiled them
  (below).

Refused: a trait-object variant per root (an `Arc` per cell read breaks
`a_leaf_cell_read_allocates_nothing`, every compare and hash a vtable call,
and it cannot answer `Value`). Refused: plain leaves with the identity in
metadata (it moves the value-stream bytes, the digest feed and the ranks); it
stays the fallback an *unregistered* `yggdryl.<name>` takes at Arrow import.

### D3, `DataTypeId`

Decided: a newtype `pub struct DataTypeId(u8)` with derived `Clone, Copy, Eq,
PartialEq, Ord, PartialOrd, Hash` and the core identifiers as associated
consts under their CamelCase names (`DataTypeId::Int32`), so a const stays
usable as a pattern through `PartialEq` (structural match on a derived-`Eq`
newtype). `kind()` reads the byte's family range. `Debug` and `Display` print
the name. `as_str` and `arrow_extension_name` are not `const` for a
registered byte: they read the register (an O(1) array by byte). The `const`
contexts that read a name today - `ExtensionType::NAME`
(`arrow/extension.rs:57`), the `FAMILY` consts in `typed.rs:1012` and
`value/mod.rs:1088` - read the kind's own const (`Isin::NAME`,
`Isin::EXTENSION_NAME`).

The 21 market ids leave `DataTypeId`: `DataTypeId::Isin` ... `::TimeInForce`
are deleted in S1 and each kind answers `Isin::ID` (`MarketKind::id`). The
243 + 15 lines are re-spelled by one script. `DataTypeId::ALL` (96 after P0)
becomes the core's own 75 identifiers; `DataTypeId::all()` answers the core
ids plus the claimed bytes in today's declaration order (the codes between
`MediaType` and `Uuid`, the enums after `State`), which is what the two
bindings list, so `DATA_TYPE_IDS` and `dataTypeIds` keep 96 members in
today's order.

Evidence: the spike (below). Refused: keeping the enum with the market bytes
as reserved members, which keeps the core naming every code and leaves no way
for a crate to add one.

**Spike result (S0).** `scratchpad/s0_spike.py` rewrote the enum in
place as `pub struct DataTypeId(u8)` with one `pub const <Name>: Self =
Self(<byte>)` per identifier (96 written), `as_u8` reading the field and
`Debug` writing `as_str`, and ran `cargo check -p yggdryl --lib
--keep-going --message-format=short` on the P1 tree (`08ae4c6b7`), then
restored the file. One diagnostic in the whole crate: `E0004` at the
`match self` of `DataTypeId::as_str` (`datatype_id.rs`), the one
exhaustive match over the identifiers, which the newtype turns
non-exhaustive (`DataTypeId(1_u8..=8_u8)` and the other gaps the retired
bytes leave). No `as u8` cast, no second exhaustive match and no
type error anywhere outside the file: the crate type-checked clean before
match checking ran, so every consumer reaches an identifier through
`as_u8`, `from_u8`, a const pattern or a `DataTypeKind` range already.
S1's sweep for D3 is therefore the file itself - the enum, `as_str` and
`from_u8` as a table over `ALL` rather than a match - plus whatever the
test targets and the bindings' `match` arms add, which the `--all-targets`
check of S1's build row lists.

### D4, what of the market set the core keeps

Decided as the prompt says. `State` stays a concrete core leaf (`TextLine`
holds one and implements `graph::Event`; the text row is laid out from
`ElementColumn::ALL` and `EventColumn::ALL`), `enum_leaf!` serves it alone
from S1. `graph/element.rs`, `graph/column.rs`, `graph/element_column.rs`
stay in the core as `yggdryl::graph`, the event vocabulary;
`yggdryl_market::graph` re-exports none of it. `State::from_fix_status`,
`FIX_STATUS_TAGS` and `from_fix_msgtype` move to `fix/` as free functions in
S3 and to `yggdryl-fix` in S4, with the Python classmethods redirecting in S3
and moving to `yggdryl.market.fix` in S5, no alias. All 17 codes are market.

### D5, constructor spelling (S3)

Decided: the market type's own associated items - `Isin::dtype()`,
`Isin::field(name)` (or whatever word the `dtype` rename, out of scope here,
has reached by then) - replace the 21 `impl DataType` blocks the kind files
hold (`marketdatakind.rs:278`, `marketdatatype.rs:546` and
`timeinforce.rs:140` beside the eighteen code ones); no extension trait preserves `DataType::isin()`. `DataType::is_code`,
`code_width` and `code_name` stay in the core reading the descriptor. S1
leaves the constructors where they are, S3 applies the spelling in place.

### D6, the implementer surface

Decided: one documented public module, `yggdryl::implementer` (a root file
`implementer.rs`; the name says who it is for and nothing it holds is API a
caller reaches for), of forwarding `pub fn`s and `pub use`s for exactly the
items a crate outside the core needs, with its own `.api-inventory.txt`
section. `Proof::Proven` never crosses it; only an `Unproven` landing door
may. The mechanism, item by item: a `pub fn` forwards a function; a `pub
use` republishes an item a privately declared module holds; a crate-private
*type* a moving crate names (`InstantSequence`, `pub(crate)` at
`graph/element.rs:791` inside the published `pub mod element`, used by
`fix/{batch,codec,messages}.rs`) has its definition moved into
`implementer.rs` and the core reaches it there, because raising it to `pub`
where it sits is what AGENTS forbids; a macro (`media_serie!`,
`define_field_types!` at `typed.rs:991`, `warned!` at
`logging/warning.rs:107-121`, `delegate_event!` at `graph/mod.rs:47`) is
`#[macro_export] #[doc(hidden)]`, re-exported from `implementer`, and every
path it expands to is spelled `$crate::implementer::...` so the expansion
names nothing private - `warned!` expanding to `$crate::implementer::warn`.
Refused: a feature flag that raises `pub(crate)` items (the same API change
wearing a feature's name, AGENTS); a second copy of a type per crate. S1 publishes what S1 needs (the register's claim doors and nothing
private), S3 the market and FIX list, S6 the media list, each derived from a
scratch `git mv` plus `cargo check -p <crate> --message-format=short`.

### D7, registration

Decided.

- Install: each crate has an explicit, idempotent `install()` (a `OnceLock`),
  called from each binding's module init and the CLI's `main` before any data
  comes in; no link-time registration (`inventory`/`linkme` would add a fifth
  `unsafe` module for what `install()` already gives).
- Who installs: every public entry of `yggdryl-market` and `yggdryl-fix` that
  reads text, a store or a wire calls its crate's `install()` itself; a value
  built through a market type carries its `&'static MarketKind` and needs no
  register; each moved harness, bench and rustdoc example calls `install()`
  through one support function the S4 script inserts; a docs block that
  parses a market name through the core shows `yggdryl_market::install();`
  and `docs/types/datatype.md` says so. S1 pins the rule with the test-only
  kind: a name parsed before its claim is refused naming the missing
  registration; after the claim it resolves.
- Refusals: a claim refused names the first claimant - a byte, name or
  extension name claimed twice; a byte outside the Code or Enum range, read
  off `DataTypeKind::contains`/`range` rather than spelled again;
  a family's own number (`0x6a`, `0xc0`); a byte a core leaf holds (`0xc1`,
  `State`); a retired byte (`0x75`, `0x76`, `0x77`, `0xc6`); a `Text` storage outside
  the Code range or a `Code8`/`Code16` storage outside the Enum range. A
  lookup that finds no kind - a parsed `isin` before any claim, a serde tag,
  a value-stream byte - is refused generically ("no registered datatype
  answers `isin`: install the crate that claims it, as
  `yggdryl_market::install()` does"): the core keeps no table of market
  names to say which crate, because that table would be the second listing
  D3 refuses.
- Capacity: the code family's range `0x6a..=0x7f` holds the seventeen codes
  and three retired bytes, leaving `0x70` as its one spare
  (`datatype_kind.rs:150-156`), which S1's test-only kind takes; the enum
  range `0xc0..=0xcf` has `0xc7..=0xcf` free less the test kind's `0xc7`. A
  new code therefore needs the family's range widened in the core first -
  a planned core change, never a claim outside the range.
- Seed: until S4 the register seeds itself from the core's own 21 kinds
  before its first read and before its first registration (one `OnceLock`,
  the same claim door the crates call later); S4 replaces the seed with
  `yggdryl_market::install()` and `yggdryl_fix::install()`.

### D8, one mechanism for market and media

Decided: one root file `plugin.rs` holding (a) the keyed claim-once register,
`Register<K, V>`, a `OnceLock` over a `RwLock<BTreeMap<K, Arc<V>>>` with
`claim(key, value, crate_name)` refusing a second claim by naming the first
claimant and the crate to add, and `get(&key) -> Option<Arc<V>>`; (b) the
helper that gives a trait object clone, equality, order and hash inside a
derive-heavy enum (`DynEq`, `DynOrd`, `DynHash` supertraits with the usual
`as_any` downcast, and `impl PartialEq/Ord/Hash for Arc<dyn Trait>` through
them). The precedent is the user-function registry
(`expression/user.rs:483-494`). The market register (keyed by byte, name and
extension name; the byte key an O(1) `[OnceLock<&'static MarketKind>; 256]`
table beside the maps, because the value-stream and the cell readers read it
per value) lands on it in S1; the media register (keyed by `MimeType`,
table-format name, catalog type word and URL scheme) in S2; `LOGICAL_NAMES`
registers on it as a third key (a name to a `DataType`) in S1, never a second
registry, and the listings D10 moves (`StringEnum::PREBUILT`,
`DataType::CODES`) on a fourth, keyed by the listing's name. `pub const
DataType::LOGICAL_NAMES` becomes `DataType::logical_names()` reading the
register - a public spelling that moves, read by `python/src/datatype.rs:1126`
and `node/src/datatype.rs:323`, both re-spelled in S1 with the binding doors
unchanged. Refused: one registry per domain (D8 exists so the market and the
media share one claim-once mechanism and one refusal). Not shared: the trait, the key, a `Serie` variant - a medium needs
none, `Serie::GenericMedia` already carries any medium type-erased.

### D9, the cycles and protocol views (S3)

Decided as recommended: `yggdryl-market` defines the trait a message that
splits into market leaves answers (walk and merge as itself,
`into_market_data`, `into_market_leaf`, its stable hash); `MarketData::Fix`
holds that trait object and `yggdryl-fix` implements it; `MarketKind::Fix`
stays, spelled `fix`. The core publishes a protocol-view builder (the
`protocol_field_types!` shape as a macro a crate invokes for its own scheme)
and the FIX view with its `FIX:` keys moves whole to `yggdryl-fix`,
`Field::as_fix` leaving the core; the same builder serves `IcebergField` in
S6. Refused unless pinned equal: deleting the `Fix` variant and splitting at
the FIX door.

### D10, the FIX vocabulary in the core (S3)

| piece | disposition |
| --- | --- |
| FIX Latest names in `LOGICAL_NAMES` | `yggdryl-fix` registers them on D8's register; `yggdryl-market` registers `mic`, `exchange` and the code names; the core keeps the non-market names. S3 first greps every reader of a FIX word outside `fix/` |
| `DateTime64::from_fix_text`/`from_fix_clock`, `Time32/Time64::from_fix_text`, the grammar in `temporal.rs:844-914` | move to `yggdryl-fix` if the ISO part delegates to the public readers with no allocation per value (the FIX parse rows in `allocations.rs` and `fix_allocations` decide); else stay public as the type's FIX spellings, with that reason recorded by S3 |
| the FIX entry of `for_each_well_known_protocol!`; the `FIX:*` keys | move with the view (D9) |
| `Scheme::FIX` | moves if it names only the protocol; stays if the URI vocabulary routes on it (S3 reads `scheme.rs:27, 88, 169, 210, 420`) |
| `MimeType::{FIX, FIXUL, FIXML, ULLINK}`, the frame classifier | stay as routing vocabulary and the one bounded content read; no medium routes them (`Media::open_as` has no arm; only `mime_type/line.rs` and the bindings' enum tables name them), so nothing registers for them |
| `parallel.rs` | stays, generic; `ordered` reached through D6; its AGENTS row restated without FIX |
| logging targets | the facade maps `yggdryl_<crate>::` to `yggdryl.` in one place (`logging/facade.rs:72-76`, whose `strip_prefix("yggdryl")` would otherwise leave `_fix::` and file every moved record as foreign), so the 59 `warned!` sites (28 in `fix/market.rs`) - which pass `module_path!()` as target and deduplication key - and about 120 `log` sites (`fix/` 81, `iceberg/` 29, `graph/` 4, the ISIN registry 6) keep every logger name byte-identical with no edit; the deduplication key is the mapped name; pinned in `rust/tests/logging/` with a record from a `yggdryl_fix::` module. Refused: an explicit `target:` on every site (about 180 edits, and `warned!` has no target argument) |
| `StringEnum::{CURRENCIES, COUNTRIES, MICS, SIDES, TIMESINFORCE, PREBUILT}`, `DataType::CODES` | registered listings that move with their codes (S1 makes them answer the register; S4 moves the tables); `generate_fix_dictionary.py` writes only `fix/constants.rs`, re-pointed in S4 |

### D11, the binding boundary

Decided: option (d), split the Rust crates only; one native module per
runtime links every crate and registers the market classes as today; the
market packages are pure language layers. Refused: (a) a C ABI/PyCapsule
(hand-written `unsafe` the workspace denies), (b) two native modules
(`isinstance` breaks, every global doubles, `DataType("isin")` cannot resolve
in the core copy, the artifacts double against the PyPI limit), (c) a
superset wheel (the napi type-tag hazard, a second core per platform,
import order load-bearing).

Sizes read from registry metadata on 2026-10-08, no download:

| artifact | size |
| --- | --- |
| PyPI `yggdryl` 0.1.21 | 32 files, 1,404,834,575 bytes in all: each wheel 41.3-49.5 MB (`cp311-abi3-manylinux_2_17_x86_64` 46,620,474; `win_amd64` 48,829,398; `macosx_11_0_arm64` 41,392,914), the sdist 8,530,577 |
| npm `yggdryl` 0.1.21 | `dist.unpackedSize` 336,971,910 |

Option (d) leaves these unchanged; whether that is acceptable is a question
for the user (the handoff carries it). Python namespace: `yggdryl/__init__.py`
gains `__path__ = __import__("pkgutil").extend_path(__path__, __name__)`;
`python/market/yggdryl/market/` carries no `yggdryl/__init__.py`; whether
`mypy --strict` merges the namespace is proven in S5, and if it fails
`yggdryl/market/` ships inside the core wheel and the user is asked. FIX ships
inside the market packages: nothing found argues for separate FIX packages
(one native module either way, and FIX's Python and JavaScript facades are
small). The pure-Python build backend is S5's choice. The generated stubs
describe the one native module and cannot split: `python/yggdryl/_native.pyi`
and Node's `index.d.ts` stay whole in the core packages, and only the
facades (`python/market/yggdryl/market/*.py` with their `.pyi`, the market
JavaScript files) split - a correction of the prompt's "the stubs split".

### D12, names

Decided: crates.io `yggdryl-market`, `yggdryl-fix`, `yggdryl-avro`,
`yggdryl-parquet`, `yggdryl-iceberg`; PyPI `yggdryl-market` imported as
`yggdryl.market`; npm `yggdryl-market` (recommended: one string preflight
checks on all three registries), the user's `yggdryl.market` the valid
alternative - put to the user before S5 publishes. Availability, read-only
on 2026-10-08: crates.io none of the five names exists (`cargo search`, exact
match absent, the search itself proven by `yggdryl = "0.1.21"`); PyPI
`yggdryl-market` answers 404 (`pip index versions` finds no distribution);
npm `yggdryl-market` and `yggdryl.market` both answer 404. No check was
skipped.

### D13, AGENTS.md and docs layout

Decided: one root `AGENTS.md` with a section per crate (a nested file is not
injected by every harness); one mkdocs site; the generated Rust docs target
moves in S4 to `cli/tests/docs_examples.rs` (git-ignored), run as
`cargo test -p yggdryl-cli --test docs_examples` with the runner's features,
`RUST_TARGET`, `run_rust` and `.gitignore` moving with it.

### D14, fixtures after S4

Decided as the prompt says: core tests, benches and doctests that use a code
as a fixture move onto a test-only registered kind in `rust/tests/support/`
(the one S1 creates for `market_register.rs`, shared from S4); tests that pin
market behaviour move to the market crate; `rust/tests/fix/ulbridge.log`
moves to `rust/tests/support/ulbridge.log` with every reader re-pointed and
no corpus change. The 90 comment lines are read one by one in S4
(`txhash/value.rs:195`'s "Unit" is English; `crate::graph::Event` stays).

### D15, s3tables

Confirmed: `s3tables/` is a feature of `yggdryl-iceberg` implying
`yggdryl/s3`, not a crate; `iceberg/table.rs` calls `s3tables::locate`,
`create` and `open_or_create` at its three location doors, so two crates
would form a cycle.

### D16, Avro in the core

Confirmed: the core decodes no Avro - value streams and pickles ride
`variant.rs` (`valuestream.rs`, `variant.rs`), and `snap` is pulled by the
`parquet` feature alone (`rust/Cargo.toml`: `parquet = ["dep:parquet",
"dep:snap"]`). `yggdryl-avro` carries `snap` itself from S6.

### D17, `iceberg/types.rs`

To confirm in S2: the default core reaches `yggdryl::iceberg` through
`#[path = "iceberg/types.rs"]` without the feature (`lib.rs:79-81`); the core
keeps the Iceberg type-string spelling (`parser.rs`) and
`compatibility::Target::Iceberg`, the rest leaves in S6.

### D18, S8a's prerequisites

Deferred to S8a, stated whole in the prompt.

### D19, the hashes: nothing moves

Decided: every stored hash and every feed is byte-identical through S1.

What feeds what today, read off the tree:

- `Scalar`'s std `Hash` (`scalar.rs:1202`) writes `value_rank` as a `u8`,
  then for a code `code_key(self).hash` - the `DataTypeId`'s derived hash,
  which on a `#[repr(u8)]` field-less enum writes the discriminant as one
  `u8` (its `DiscriminantKind::Discriminant` is the repr type), then the
  `SmolStr` as a `str` (its bytes, then `0xff`) - and for an enum member the
  derived hash of the `#[repr(u8)]`/`#[repr(u16)]` leaf: its code as one
  `u8`/`u16`. `stable_hash_of` (`hashing/stable.rs`) is XXH3-64 over
  little-endian integer writes.
- `DataType`'s std `Hash` writes `Shape::of(self).hash`; `Shape` has no
  `repr`, so its derived hash writes the variant's position as an `isize`
  (`write_isize` -> `write_i64` LE). The 72 live positions in declaration order
  (`datatype.rs:761-844`): `Null` 0 ... `String` 22, `Country` 23, `Ccy` 24,
  `Mic` 25, `Cfi` 26, `Isin` 27, `Side` 28, `State` 29, `TimeInForce` 30,
  `Uuid` 31 ... `MediaType` 55, `Cusip` 56, `Sedol` 57, `Bbg` 58, `Figi` 59,
  `Unit` 60, `Decimal` 61, `BigDecimal` 62, `Ric` 63, `MarketDataKind` 64,
  `MarketDataType` 65, `Forex` 66, `Lei` 67, `Bic` 68, `Elf` 69, `Dti` 70,
  `Fisn` 71; position 72 was `PluginSide`, retired by P0 and never reused.
- The doors that persist a std-hash-derived digest: `FixRegistry::stable_hash`
  (the dictionary pin `14_542_711_836_201_211_247` since P0, `17_114_512_833_162_386_024` on `2ae975674`), `FixMsg::stable_hash`,
  `RecordOptions::stable_hash`, and the `internals` doors
  `scalar::stable_hash_of` and `hashing_stable::stable_hash_of`. Python's
  `Scalar.__hash__` is **not** one of them: it is `Scalar::stable_hash`
  (`python/src/scalar.rs:1607`, `xxhash/scalar.rs:257`), the canonical XXH3
  feed, so its pin (`python/tests/test_scalar.py`) is a canonical-feed pin;
  the std-hash value pins go through `yggdryl::internals::scalar::stable_hash_of`
  under the `internals` feature, in each kind file's `internal` module (a
  finding against the prompt's wording, recorded here).

`DataType`'s `Ord` is a third per-kind table, `dtype_rank`
(`datatype.rs:647, 1001-1089`), neither the byte nor the `Shape` position,
and it is caller-visible: Python's `DataType.__richcmp__`
(`python/src/datatype.rs:2193-2197`) and Node's `DataType.compare`
(`node/src/datatype.rs:870`) read it, and by byte Lei (`0x6b`) would sort
before Country (`0x71`) where by rank Country (30) sorts first. The ranks of
the kinds and the core leaves between them: `Country` 30, `Ccy` 31, `Mic`
32, `Cfi` 33 (`Uuid` 34), `Side` 53 (`Geography` 52), `State` 55,
`TimeInForce` 56 (`Url` 57), `Isin` 58 (`Timezone` 59, `MimeType` 60,
`MediaType` 61), `Cusip` 62, `Sedol` 63, `Bbg` 64 (`Urn` 65), `Figi` 66
(`SortedMap` 67), `Unit` 68 (`Decimal` 69, `BigDecimal` 70), `Ric` 71,
`Forex` 72, `MarketDataKind` 73, `MarketDataType` 74, `Lei` 75, `Bic` 76,
`Elf` 77, `Dti` 78, `Fisn` 79; 80 was `pluginside`, retired by P0. S0 pins
this order across the kinds and against those neighbours in
`rust/tests/root/datatype.rs` (a review finding).

The fix in S1: each `MarketDescriptor` states its value rank (`18` for every code;
`27` State stays core; `28` MarketDataKind, `29` Side, `30` MarketDataType,
`31` TimeInForce; rank `32` was `PluginSide`, retired by P0), its datatype
rank (the table above; `DataType::cmp` reads it for `DataType::Market`, a
kind claimed later taking the next free number, 81) and its `Shape` position
(the numbers above). `Scalar`'s `Hash` for `Scalar::Market` writes the rank `u8`, then for
a `Text` value `state.write_u8(byte)` and the text as `str`, for a `Code8`
value `write_u8(code)`, for a `Code16` value `write_u16(code)`: exactly what
today's variants feed. `Ord` reads the rank, then for codes `(byte, text)`,
for an enum its code. `impl Hash for DataType` writes
`state.write_isize(position)` for `DataType::Market`; `Shape` keeps every
other position under `#[repr(isize)]` with explicit discriminants equal to
today's numbers (an enum with fields needs a primitive repr for explicit
discriminants, E0732; `isize` is the width the derived hash writes today). A
kind claimed later takes one reserved rank (codes `18`; enums the next after
`32`, i.e. `33`) and one reserved `Shape` position (`73`), then orders and
hashes by its byte; a core variant appended later takes position `74`
onward. The dictionary pin is unmoved in S1; the canonical XXH3 feeds
(`xxhash/scalar.rs:83-91, 348-361`) never move.

### D20, typed market values

Decided: an owned narrowing. The core gains `MarketValue` in `value/`, the
contract a registered kind's typed value answers: `const KIND: &'static
MarketKind`, `fn into_scalar(self) -> Scalar` and `fn from_scalar(value:
&Scalar) -> Option<Self>` (owned: a `Str` clone is inline below 24 bytes and
one `Arc` bump past it; a `u16` is `Copy`). `CodeValue` and `EnumValue` keep
their per-type consts and readings; the market enums answer `MarketValue` and
`EnumValue`, `State` answers `Value` and `EnumValue` (D4). `Value` is
unchanged, the borrowed `from_scalar` staying the contract every core leaf
answers - and the supertraits are restated so a registered kind can answer
them (`value/mod.rs:185` declares `CodeValue: Value`, `:267` `EnumValue:
Value + Copy + Default`): a market enum answers `Value::from_scalar` by
borrowing the `&'static` member out of its table (`Self::ALL`), so
`EnumValue: Value` stands; a code cannot borrow an `&Isin` out of a
`Scalar::Market` holding a `Str`, so `CodeValue` drops `Value` as its
supertrait and requires `MarketValue` instead, its readings (`rank`,
`is_real`, `is_canonical`) unchanged. The AGENTS `value/` row says so. The marker `TypedField` takes for a registered kind is the kind's
own ZST (`IsinType`, today's `define_field_types!` output, kept because
arrow-rs's `ExtensionType` is implemented on it) whose `DataTypeValue::id()`
is `Isin::ID` and `into_dtype()` is `DataType::Market(MarketType(&ISIN))`;
`MarketType` is the family marker beside it (`FieldOf<MarketType>` is the
leaf). AGENTS rules that change: "one sealed marker per datatype variant"
gains "and one per registered kind over the `Market` variant"; the `Value`
row of the `DataType`, `Field`, `Scalar` table gains "a registered kind's
typed value answers `MarketValue`, an owned narrowing". Refused: a borrowed
one-pointer view per kind (`IsinRef<'_>`), a second type beside each `Isin`
for a clone that is inline.

### D21, the media point's contract (built in S2)

Decided. One trait per role, each registered on D8's register:

| role | key | trait, the methods it carries | replaces |
| --- | --- | --- | --- |
| codec | `MimeType` | `MediaCodec`: `read_batch_reader(handle, declared, options)`, `row_size`, `read_field`, `overwrite_arrow_reader`, `read_stream` (the native row stream, `None` where the medium has none), `open(handle) -> Box<dyn IOMedia>` (the stateful wrapper) | `iobase/transfer.rs:1171-1290`, `iomedia.rs:1479-1490`, `Media::open_as`, `Holder::into_media_base`, `require_kind`, the outer-coding refusal (`compresses_internally()` on the trait) |
| table format | name (`"iceberg"`) | `TableFormat`: `locate(handle) -> Option<Box<dyn LocatedTable>>`; `LocatedTable`: `is_whole`, `overwrite_whole`, `clear`, `stored_field`, `read_arrow_field`, `read`, `overwrite_prepared`, `append_prepared`, `merge_prepared` | the 11 `iceberg::located` sites, `Located` (`iceberg/mod.rs:146-318`), the write-session hooks, `WriteCount::skip` |
| catalog factory | the `type` word and the scheme | `CatalogFactory`: `catalog(name, url, properties) -> Catalog` | `warehouse/catalog.rs:95-139` |
| locator | URL scheme | `Locator`: `holder(location, properties) -> Holder` | `Holder::from_url`'s per-scheme arms (`holder/mod.rs:364-376`) |
| capability | - | `IOMedia::as_any(&self) -> &dyn Any` | `parquet_footer` on `IOMedia` (`iomedia.rs:369-409`); `read_parquet_statistics` and `read_parquet_geospatial_statistics` take any handle and route by media type through `parquet_leaf(self)`, which no downcast of a `Holder` answers, so they move to `yggdryl-parquet` as free functions over `&dyn IOMedia` and leave no Parquet-named method on the trait |
| options | - | `RecordOptions::Registered(RegisteredOptions)`: the 14 shared sections `record_options_fields!` writes, the `MimeType`, and the medium's own settings as `Arc<dyn MediumOptions>` (`DynEq + DynOrd + DynHash`, D8) | the per-medium variants of the moved media, their accessors moving onto their media |

`RecordOptions::stable_hash` is a persisted feed (D19), so the registered
form hashes exactly what today's variant does: `stable_hash_of(&(tag,
settings))` with the medium's tag (`"parquet"`, `"avro"`, ...) and the
settings struct's derived `Hash` in today's field order, through `DynHash`;
the `Ord` and `Hash` of the enum itself keep today's variant positions. S2
pins every medium's `RecordOptions::stable_hash` value before its first
edit, as S0 pins the market kinds.

The pushdown rides the options, not the trait: every door hands the medium
its `RecordOptions` whole and the medium reads today's pushdown through the
published section readers - `apply_columns`, the `where` section with
`filter_phases` and `apply_arrow_expressions`, `partition_pairs`,
`max_row_size`, `row_offset`, `limit_arrow_reader`; `Bounds` and `Bound`
published through D6. No pushdown argument rides a trait, so S8 changes the
readers and the `MediaSerieValue` defaults, never the point. Refused: a
per-medium arm left in the core; a second register; a pushdown argument added
now for S8.

### D22, `pluginside` deleted (P0)

Decided by the user mid-S0 ("Delete the pluginside type, reuse the side")
and landed as its own commit before S0's pins, so S0 pins 21 kinds plus
`State`. A FIX plugin's role is the side of the market its session stands
on, so it is a `Side`: `FixSource::pluginside` and the fixed row's
`msgpluginside` (tag 65043) are typed `side`, the intrinsic
`msgpluginsidecodeset` renders `Side::ALL` (nineteen members, the two roles
and `UKNW` among them), `yggdryl::fix::plugin_side(class)` (Python
`yggdryl.fix.plugin_side`, Node `fix.pluginSide`) reads a CBlock's class into
one and refuses nothing, and `buyside`/`sellside` join `Side`'s spellings.
Retired and never reused: `DataTypeId` byte `0xc6`, `Shape` position 72,
`Scalar` value rank 32, `DataType::pluginside()`, `PluginSideField`, the
Python `PluginSide` enum and factory, the Node `PluginSide` object,
`pluginSideFromPluginType` and `fields.pluginside`, the page
`docs/types/enum/pluginside.md`. The committed field shard and codeset were
re-dumped, and the dictionary hash re-pinned with its sentence.

### D23, the medium holds its options (P1)

Decided by the user mid-S0 ("make the media directly hold options instead
of the serie, as it is its role to tell or infer correct options with
defaulting"), mapped by a five-reader workflow and one synthesis over
`media_serie.rs`, the eleven `media_serie!` media, `iomedia.rs`, the
bindings and the pins. Today the scan's `RecordOptions` live twice: the
seven wrappers (`Ipc`, `Parquet`, `Avro`, `Csv`, `Text`, `Excel`, `Xmla`)
hold their encoding's options and answer them from `record_options()`, a
bare handle, `IcebergTable` and `MediaTable` work them out on every call,
and `MediaSerieState<T>` takes its own copy at construction
(`media_serie.rs:18, 65, 470`), which every read passes (`:92-99`), every
verb clones and rewrites (`:120-129, 181-194, 261-292`) and `read_options`
lends (`:253`); the five-clause clearing list is written four times
(`iomedia.rs:1110-1115`, `media_serie.rs:67-72, 141-146, 505-509`); and a
serie built with explicit options read under them but wrote under the
medium's (`write_serie`, `:412`). No caller passes `Some`: every construction
site is a test (`rust/tests/root/media_serie.rs`), nothing in `rust/src`,
the bindings or the CLI builds a media serie, and no `read_serie` answers
one.

The shape. The medium holds the options and is asked at every read; the
serie keeps no `RecordOptions`, only what its own verbs stated - a
crate-private `Scan { filter, select: Option<Selector>, range: Option<(u64,
Option<u64>)> }`, `with_filter` and `with_key` conjoining the predicate,
`with_select` restating the selection, `with_row_range` the range - laid over
the medium's answer under the one guard a read already takes (`compose`:
`record_options`, the clauses, `require_write_limits`). `MediaSerieState::new(media)`
and every `XSerie::new(media)` take the medium alone: a caller states options
on the medium (`Csv::new(..).with_options(..)`, as every test already does)
and a medium holding none infers them - the encoding from the media type
(`iomedia.rs:278-305`, `RecordOptions::for_media_type`), the field from the
medium's own schema read, the write limits a check and never an inference.
Construction costs one `record_options` and one `read_arrow_field`, a
read, a re-plan or a seek of one cell one `record_options` more - nothing
of the store on a wrapper, `media_type=1 is_container=1` on a bare leaf
handle, as the handle's own `read_serie(None)` costs - stated in a new
`iobase_calls` row (`a_media_serie_asks_its_medium_for_the_options_on_every_read`:
built `pstream_bytes=1 media_type=2 is_container=2`, re-planned
`media_type=1 is_container=1`, drained, filtered and drained, and one
cell sought each `pstream_bytes=1 url=1 media_type=3 is_container=2
parent=1`). A re-plan is `with_scan` - the clauses replaced, nothing
asked - then the leaf's `from_media_state`, which is `MediaSerieState::bound`:
the one ask of the medium, judging the encoding under the leaf's
`require_media_options` and binding the field under the composed options,
so the public door's refusal of a medium of another encoding and the
re-plan's binding are one `record_options`, never two. A medium that
infers its options - a bare container handle listing its children
(`iomedia.rs:278`), an Iceberg location probing its `metadata/` - pays
that inference on every ask, which is the medium's own cost and the
reason a caller holding such a handle states the options on a typed medium
(`Media`, `Ipc<H>`, `Csv<H>`) once. An edited
snapshot (`splice`, `slice`) resets the clauses and reads under the medium's
options with the five clauses it already answered taken off through the one
owner of that list, `iomedia::dimensions` (the function `dimension_options`
now calls), its field declared; `scalar(i)` composes per seek and stores
nothing. `read_options` is deleted with its two pins; `Debug` prints the
scan.

Refused: a setter on `IOMedia` (the serie never writes options to its
medium; the per-call property copy of §3 and `docs/media/index.md:357` is
the bindings' rule and would be contradicted; no caller needs one - the
wrappers' own `with_options`/`options_mut` stand; speculative generality).
Refused: a clause set held as a `Plan` applied with `set_plan`, which clears
every section it leaves out, the declared field included (`plan.rs:2368-2371`
restores it), where `with_filter` must conjoin. Refused: a cloned medium per
derived scan - `Box<dyn IOBase>` has no clone and `Holder::from_handle`
strips every wrapper and refuses a buffer - and a setter under the shared
mutex, visible to every sibling scan and discarding the opened footer and
schema caches (`parquet/mod.rs:2133`, `ipc/mod.rs:816`, `avro/batch.rs:2460`)
on every re-plan. Bindings: unchanged, no door reaches a media serie; Node
gains nothing. Pins: `rust/tests/root/media_serie.rs` re-spelled to
`new(media)`, a clone's independence after a sibling re-plans added, the
`iobase_calls` row added; the deadlock pin and the row-pull counts unmoved.
For the split, D21's options role reads the same way: the medium's own
settings ride the registered options and the serie states nothing of them.

### D24, `yggdryl-excel` (S6b)

Decided by the user mid-S0 ("Isolate also the yggdryl-excel crate"). The
workbook medium is a medium like Avro and Parquet and leaves the same way,
through D21's media point: `excel/` is 7,349 lines in twelve files and names
nothing of the core a medium may not - `zip::ZipArchive` (the OPC package),
`holder::{Buffer, Holder}`, `arrow::{BatchReader, arrow_schema_from_field,
field_from_arrow_schema}`, `media::{IORecordOptions, RecordOptions,
DEFAULT_ROOT_NAME}`, `Serie`, `StreamChunkedSerie`, `ArrowCastOptions`,
`Str`, `TemporalKind` and the value types (`git grep -h '^use crate::' --
rust/src/excel | sort -u`, 16 distinct lines). The core names it on 39
lines outside `excel/` (`git grep -n excel -- rust/src ':!rust/src/excel' |
wc -l` at HEAD): the `RecordOptions::Excel(ExcelOptions)` arm and its 28
dispatch arms in `media/options.rs:1359` and `media/options/dispatch.rs`,
five `RecordOptions::Excel` arms in `iobase/transfer.rs:1189-1318` (the
read, the row size, the field, the overwrite and the stated field), the
`Media::Excel` variant and its arms in `media/mod.rs:93-563`,
`Serie::Excel(Arc<ExcelSerie>)` at `serie.rs:658`, `holder/mod.rs:815`'s
routing, `lib.rs:60` and the two `mime_type` registry lines - every one a
site the media point already has to erase for the three S6 media, so S6b
is the same sweep over one more medium. `TemporalKind` and `Str` are
crate-private today and cross through D6's implementer module (S6's list
gains them). The binding crates keep their `python/src/excel.rs` and
`node/src/excel.rs` as they keep Avro, Parquet and Iceberg (U4, D11 option
d): the one native module links `yggdryl-excel`. S6b is its own commit
after S6 because Excel shares none of the Avro-Parquet-Iceberg cycle that
makes S6 one commit, so a tree with Excel still in the core builds; S2
claims Excel through the media point beside Avro, Parquet and Iceberg - the
`RecordOptions::Excel`, `Media::Excel`, `Serie::Excel` and `transfer.rs`
arms are erased there, in place - so S6b is a move only, as every S6 move
is. The name: crates.io answers 404 for `yggdryl-excel` (read 2026-10-08
through the crates.io API; `cargo search` answered nothing through the
sandbox proxy in 60 s and was not relied on); release preflight and AGENTS
§6's name list gain it in S6b. `Str` is public (`string.rs:55`, `lib.rs:343`),
so it crosses nothing; `TemporalKind` and the crate-private items Excel
reaches by full path - `iobase::{owned_handle, leaf_writer,
append_arrow_reader_default, merge_arrow_reader_default,
overwrite_arrow_reader_default_with_field}`, `iomedia::{own_options,
dimension_options, container_field, container_row_size, read_record_serie}`,
`xml::{write_element_text, write_leaf_text, write_x_escape,
write_attribute_text, decode_x_escapes}`, `text::{prepare_text,
expected_got, elide_to, ERROR_TEXT_LIMIT}`, `temporal::{parse_date,
parse_time, format_datetime, scalars}` and `media_serie!`, about 25 - are
orientation; S6b derives the exact D6 list from the scratch `git mv` plus
`cargo check`. Refused: folding Excel into S6's
commit (a larger tree to review for no reason S6's rule gives); leaving
Excel in the core (the user asked otherwise; it has the medium's own
options, serie and interop, and nothing in the core but the routing
vocabulary needs it).

## S4's shape

One commit. The moves are `git mv` with path rewrites, which git shows as
renames, so the reviewable diff is the rewrite script's output; the two-step
shape (S4a everything into `rust/market`, S4b FIX out) re-points the
bindings, docs and inventories twice for no second tree that builds.

## S6's shape

One commit in dependency order inside it (avro, parquet, iceberg). The
reversed S6a-c is the only buildable split and is put to the user by the S5
handoff if S6's diff is too large; it is not taken without the user.

## Risks

- D19: a stored hash moves if S1's `Hash` impls feed one byte differently;
  the pins catch it, and a move is a defect, never a re-pin.
- D11: `mypy --strict` over a `pkgutil` namespace is unverified until S5.
- S4 and S6 diff size: reviewed from the rewrite script, else the two-step
  shapes.
- The instrument-registry change planned outside this repository will rebase
  onto the paths S4 moves (`rust/market/src/isin_registry.rs`,
  `rust/market/src/isin_registry/`).
- The code family has no spare byte once S1's test kind takes `0x70` (D7):
  a new code is a core range change first.
- The first publish of six crates.io names, one PyPI name and one npm name:
  preflight refuses a half-out version, and the registry configuration (PyPI
  pending publisher, npm first publish, `CARGO_REGISTRY_TOKEN` scope) is the
  user's before the merge.
- CI time: the workspace already runs two feature lanes; each new crate's
  suite adds to the `rust` job.

## The slice table

| S | commit | proved by |
| --- | --- | --- |
| P0 | `pluginside` deleted, the plugin's role a `Side` (U8, D22): "Delete PluginSide; the plugin's role is a Side" | the whole run in the all-features lane; both bindings' suites; the three example passes; CI read |
| P1 | the medium holds its `RecordOptions` (U9, D23) | the media suites and their `iobase_calls` rows; both bindings; CI read |
| S0 | design, the pins, the prompt and this file: "Pin the market kinds' wire and cost contracts" | pins generated and green on P1's tree; pushed; CI read green |
| S1 | the market extension point; the four enum kinds claimed in place, the seventeen codes the core's flat variants (D25) | S0's pins byte-identical; the cost and size gates; `rust/tests/market_register.rs` |
| S2 | the media extension point (D21), the media claimed in place | `iobase_calls` unmoved; every medium's harness; both lanes; the exchange jobs |
| S3 | the remaining seams in place: D5, D6, D9, D10 | `cargo check --workspace --all-targets`; the dictionary hash and crate dump unmoved; the whole run |
| S4 | `yggdryl-market` and `yggdryl-fix` | market and fix whole runs in both lanes; cost rows unmoved; `cargo package --list`; maturin sdist |
| S5 | the market binding packages; release and CI learn them | the packages' suites in CI; a rehearsal on the user's go |
| S6 | `yggdryl-avro`, `yggdryl-parquet`, `yggdryl-iceberg` with `s3tables` | each crate's suite; the exchanges; the Parquet `iobase_calls` rows; the MSRV job |
| S6b | `yggdryl-excel` | its suite in both lanes; the Excel exchange job (openpyxl); `rust/tests/interop/excel.rs` no longer skipping |
| S7 | `RecordOptions` -> `MediaOptions` | the sweep's grep empty; every cost pin unchanged |
| S8a | D18's prerequisites | S8's pins |
| S8b | the planned expression series | S8's pins |
| S9 | the final sweep | the user's go |

## S0: what was pinned

The pins live where the prompt says: a kind's own facts in
`rust/tests/root/<kind>.rs` (`the_<kind>_wire_contracts_are_pinned`, with the
std-hash feed in that file's `internal` module under the `internals`
feature), the cross-kind order and the dictionary order in
`rust/tests/root/scalar.rs`, the identifier listing beside the existing byte
pin in `rust/tests/root/datatype_id.rs`, the unknown-extension fallback and
the retired-name refusal in `rust/tests/root/field.rs`, the cost gates in
`rust/tests/allocations.rs`, and the binding facts in
`python/tests/test_scalar.py`, `python/tests/test_version.py` and
`node/tests/fields.test.js`. The literals were generated by one scratch
harness (`rust/tests/s0_facts.rs`, run once on `2ae975674`, once more on
P0's tree - every fact of the 22 kinds identical, P0 having moved none -
and deleted before the commit) so that no value was typed by hand. Every
pin builds its values through the doors that survive S1 - `DataType::from_str`,
`DataTypeId::from_u8`, `scalar(text)`, serde, the value stream, Arrow -
never a `DataType::Isin`, `DataTypeId::Isin` or `Scalar::Side(..)` spelling,
so S1's worker J never edits them; the one re-pin S1 is allowed in
`rust/tests/root/datatype_id.rs` is the existing `DataTypeId::ALL` length
(96) and the zip over it in `every_discriminant_is_stated_and_pinned`,
because D3 shrinks `ALL` to the core's 75 - the listing pin S0 adds reads
every byte through `from_u8` instead. The `DataType` order across kinds and
against their core neighbours (D19's third table) is pinned in
`rust/tests/root/datatype.rs`. The baseline counts re-run on P0's tree:
1,297 lines name the 22 kinds in what stays (1,296 the 21; 1,299 the 21 on
`2ae975674`), 232 `DataTypeId::<kind>` lines, 351 docs and skills lines -
each move the PluginSide deletion's.

Each kind's sample value: `country FR`, `ccy USD`, `mic XPAR`, `cfi ESVUFR`,
`isin US0378331005`, `cusip 037833100`, `sedol B0YBKJ7`, `bbg "AAPL US
Equity"`, `figi BBG000BLNQ16`, `ric VOD.L`, `forex EUR/USD`, `unit MWh`, `lei
HWUPKR0MPOU8FGXBT394`, `bic DEUTDEFFXXX`, `elf 2HBR`, `dti X9J9K872S`, `fisn
"ACME CORP/SH"`, `marketdatakind ORDR` (10), `marketdatatype ORDLIMIT`
(102), `side BUYS` (1), `timeinforce GTC` (2), `state
PENDING_NEW` (1001).

### D25, the codes stay core and flat (the user's fourth instruction, S1)

The user, mid-S1: "bring back in core all the code types/field/serie/scalar
implementations as they are generic not only market; keep most flattened
generic enums serie/scalar/datatype/field". Decided: the seventeen
registered codes (`country`, `ccy`, `mic`, `cfi`, `isin`, `cusip`, `sedol`,
`bbg`, `ric`, `figi`, `unit`, `forex`, `lei`, `bic`, `elf`, `dti`, `fisn`)
are the core's own, each a flat variant of `DataType`, `Field`, `Scalar`
and `Serie` and a const of `DataTypeId` exactly as at `6d71a36ee`, with
`CodeValue: Value`, `code_value!`, `code_scalars!`, `DataType::CODES`,
`code_for_extension` and the rest of the code vocabulary as they were; the
market extension point registers *enum* kinds alone - `side`, `timeinforce`,
`marketdatakind`, `marketdatatype` today, a crate's own later - under the
one `Market` variant of each root enum, in the Enum family's free bytes
(`0xc7..=0xcf`; `0xc6` retired). `MarketStorage` is `Code8 | Code16`, a
value is its kind and its member's code (`MarketScalar { kind, code }`),
`MarketPayload`, the descriptor's `is_canonical` and `respell` and the
`CODE_VALUE_RANK` reservation are deleted (a code's rank 18 is a core arm
again), `MarketSerie` is `Code8(Arc<UInt8Serie>) | Code16(Arc<UInt16Serie>)`,
and `DataTypeId::market(byte)` is const-refused outside the Enum range.
Refused: keeping a `Text` storage on the register for a text kind a crate
might claim later - the Code family has one free byte (`0x70`) and the user
named codes generic, so a new code is a core variant, never a claim. The
crate map moves with it: `yggdryl-market` carries the four market enums,
`graph/`, the ISIN registry and what S3's seams name, and the codes never
leave the core; U1's row is read with this correction. The S0 pins are
unchanged: every code's byte, rank, shape, hash feed, serde document and
value-stream byte is what `6d71a36ee` pinned, and the flat variants are
those pins' own spellings.

### Bench baselines (`--quick`, saved as `s0` in `target/criterion`)

**Bench baselines (S0).** Saved as the Criterion baseline `s0` under
`target/criterion/<id>/s0/` by `scratchpad/logs/chain_s0_bench.sh` on the
P1 tree (`08ae4c6b7`), release build, `--quick`, this container (the
numbers are a direction, never a page's figure): the filters the prompt
names - `types` `instrument_codes`, `^enums/`, `mic_exchange_code` and
`^ascii/`; `arrow` `arrow_serie_null_visibility`, `arrow_serie_cast` and
`chunked_stream`; `fix` and `fix_allocations` whole - 280 benchmarks
in all, each read back by `cargo bench -p yggdryl --bench <name> -- <filter>
--quick --baseline s0`. The ones the split can move, median of the run:

| Benchmark | `s0` median |
| --- | --- |
| `instrument_codes/isin` | 68.8 ns |
| `instrument_codes/lei` | 76.8 ns |
| `instrument_codes/lei_closed` | 128 ns |
| `instrument_codes/dti` | 134 ns |
| `instrument_codes/dti_closed` | 95.4 ns |
| `instrument_codes/bic` | 66.5 ns |
| `instrument_codes/elf` | 54 ns |
| `instrument_codes/figi` | 75.4 ns |
| `instrument_codes/fisn` | 81.2 ns |
| `instrument_codes/ric` | 48.6 ns |
| `instrument_codes/ric_exchange_code` | 19.4 ns |
| `instrument_codes/forex_new` | 273 ns |
| `instrument_codes/forex_from_symbol` | 339 ns |
| `instrument_codes/cfi_classification` | 18 ns |
| `instrument_codes/cfi_refine` | 45.3 ns |
| `instrument_codes/market_identifier_setter` | 267 ns |
| `enums/marketdatakind_ingest_int` | 68.6 µs |
| `enums/marketdatakind_ingest_utf8` | 150 µs |
| `enums/side_ingest_int` | 78.1 µs |
| `enums/side_ingest_utf8` | 316 µs |
| `mic_exchange_code/cta_utp` | 42.7 ns |
| `mic_exchange_code/feed_collision` | 42.7 ns |
| `ascii/scalar_from_text/ccy` | 130 ns |
| `ascii/parse_display_round_trip/ccy` | 295 ns |
| `ascii/field_arrow_projection/ccy` | 196 ns |
| `ascii/utf8_ingest/ccy` | 226 µs |
| `ascii/utf8_render/ccy` | 3.25 µs |
| `ascii/vocabulary_prebuilt` | 20 µs |
| `ascii/vocabulary_into_members` | 6.76 µs |
| `arrow_serie_null_visibility/compact_list_isin/16384` | 2 ms |
| `arrow_serie_null_visibility/compact_list_run_end_isin/16384` | 4.66 ms |
| `arrow_serie_null_visibility/compact_list_struct_run_end_isin/16384` | 3.05 ms |
| `arrow_serie_null_visibility/masked_run_end_isin/16384` | 2.85 ms |
| `arrow_serie_cast/batch/serie/16384` | 2.05 ms |
| `arrow_serie_cast/stream/serie/16384` | 2.14 ms |
| `chunked_stream/window_by/code_key/first/16384` | 1.16 ms |
| `chunked_stream/window_by/code_key/drain/16384` | 4.55 ms |
| `fix/pipeline/parse_lines` | 2.09 s |
| `fix/pipeline/decoded_lines` | 2.29 s |
| `fix/pipeline/decoded_lifecycle` | 412 ms |
| `fix/pipeline/parse_text_arrow_reader` | 3.79 s |
| `fix/pipeline/market/fix_market_arrow_reader` | 44.5 ms |
| `fix/pipeline/market/fix_book_arrow_reader` | 51.4 ms |
| `fix/pipeline/market/direct_fix_to_operation` | 5.17 µs |
| `fix/pipeline/market/book_iterator` | 5.87 ms |
| `fix/fill/known_isin` | 2.78 µs |
| `fix/fill/unknown_fisn_exact` | 325 ns |
| `fix/fill/unknown_fisn_economic` | 50.9 µs |
| `fix/ulbridge/parse` | 148 ms |
| `fix/ulbridge/parse_lifecycle` | 213 ms |
| `fix/ulbridge/market` | 212 ms |
| `fix/ulbridge/step/insert_securityid` | 448 µs |
| `fix/allocations/parse` | 26.9 µs |
| `fix/allocations/market` | 48.1 µs |
| `fix/allocations/step/insert_securityid` | 136 ns |
| `fix/allocated_bytes/parse` | 17.8 ms |
| `fix/allocated_bytes/market` | 30.7 ms |
| `fix/store/from_handle_seed` | 920 ms |
| `fix/store/stable_hash_seed_one_state_allocation` | 467 ms |
| `fix/resolve/tag_hit` | 4.53 ns |
| `fix/resolve/name_hit_folded` | 68.5 ns |
| `fix/codes/300/value_first` | 145 ns |

A slice's handoff compares its run against `s0` with `--baseline s0` on the
same filters and reports the direction; a number that moved past noise is a
design answer, never a re-pin.

## S1: what was built

The market extension point, in place: four `Market` variants holding the
core's four enum kinds, claimed by the core itself; the seventeen codes
stay the core's own flat variants (D25, the user's instruction mid-slice,
which reverted the first S1 build's claim of the codes). What the slice
decided beyond S0's decisions, each with its reason:

- **A registered kind is an enum kind** (D25): `MarketStorage` is `Code8 |
  Code16`, a value is its kind and its member's code (`MarketScalar { kind,
  code }`), the descriptor's one function pointer is `read: fn(&str) ->
  Result<u16>` (a spelling into a member's code) and its one door
  `scalar(text)` holds the reader's answer to the member table;
  `default_scalar()` is the member at code zero; `MarketSerie` is
  `Code8(Arc<UInt8Serie>) | Code16(Arc<UInt16Serie>)`; `DataTypeId::market(byte)`
  is const-refused outside the Enum family; `CORE_KINDS` are `marketdatakind`,
  `side`, `marketdatatype`, `timeinforce`; the retired byte is `0xc6` alone.
  Everything a code needs - `CodeValue: Value`, `code_value!`,
  `code_scalars!`, `DataType::CODES`, `code_for_extension`, `code_text`,
  `code_cell_text`, the per-code arms of every door, `IsinField` from the
  `field.rs` table - is HEAD's (`6d71a36ee`), verbatim.
- **`MarketValue` lives in `market.rs`**, beside the descriptor it names,
  and is re-exported at the root - not in `value/` as D20 wrote. The
  trait's one fact is `KIND: &'static MarketDescriptor`; `value/` keeps
  the contracts every core leaf answers (`Value`, `CodeValue: Value`,
  `EnumValue`). `EnumValue::KIND` and `EnumValue::EXTENSION_NAME` are
  deleted for the leaves' inherent `NAME` and `EXTENSION_NAME`, which
  `State` gains too.
- **A kind's static is `<UPPER>_KIND`** (`SIDE_KIND`), written by the
  market arm of `enum_leaf!` beside the type, re-exported at the root like
  the type; `Side::ID`, `Side::NAME` and `Side::EXTENSION_NAME` are the
  consts the static reads, so a `const` context never reads a static;
  `define_field_types!(SideType, SideField, market = SIDE_KIND, Side)`
  writes the marker and `SideField = FieldOf<SideType>`.
- **`DataTypeId` is a `u8` newtype** (D3): `ALL` holds the core's
  ninety-two consts, the seventeen codes among them after the text family
  at HEAD's bytes; `all()` splices the registered kinds in after `state`
  in byte order (ninety-six with the core's four); `as_str`,
  `arrow_extension_name`, `code_width`, `from_u8` and `FromStr` answer the
  core table for a core byte and the register for a registered one;
  `core_str` and `core_arrow_extension_name` are crate-private const doors.
- **A claim takes its three keys together** under one lock after every
  refusal is checked: a byte outside the Enum family, the family's own
  number, the retired byte, the core's `state`, a non-core claim not
  stating the reserved `(value_rank 33, dtype_rank 81, shape 73)` - so no
  rank ever meets the core's and the `unreachable!` an equal rank over
  unequal kinds would hit cannot be reached - no member, members out of
  code order or past the storage's width, an unfolded name or one the
  grammar already reads, an extension name the core recognizes; then the
  byte, the name and the extension name each against its first claimant.
  The grammar-word check is skipped for the core's own claims: the grammar
  reads the register, which is seeding while they run, and a read inside
  the seed would re-enter the `OnceLock` and hang (the first smoke run of
  the narrowed register did, in every harness test); the core's names are
  pinned as no grammar word by their own tests. The logical-name door seeds
  the market register before it takes the registers' shared lock, for the
  same reason: the seeding takes that lock, and a reader under it would
  wait on itself (the `register_logical_name` doctest did, once). The core seeds its four on
  the register's first read or claim;
  `kind_of`, `kind_named`, `kind_for_extension`, `kinds` are the readers;
  `unregistered` the refusal every intake door raises; `MarketType::validate`
  is pointer identity with the claimed descriptor, so a copy stating a
  claimed byte is no kind. `adopt_code` is crate-private, the kinds' macro
  its only caller.
- **The serde tags are read tag first, never buffered**: a derive cannot
  spell a run-time tag, and an `#[serde(untagged)]` wrapper over the derived
  shadow would buffer the document whole, which loses a 128-bit integer and
  swallows the shadow's own error messages. `Scalar`'s reader is one
  `Document` visitor that reads the `type` key, asks the derived
  `StructuralWire` whether the tag is the core's (`serde::core_tag::<Wire>`,
  one probe shared with `DataType`'s reader, recording `unknown_variant`
  and building no message), replays the key and tag into the derived reader
  for a core tag, and reads `value` through `kind.scalar` for a registered
  one; a document whose `value` precedes its `type` is the one case
  buffered. `DataType`'s reader reads the held document as the core wire
  first and, where no core tag reads it, as `{type}` through the register,
  keeping the core's own refusal word for word. Writing is by hand for a
  market value, byte-identical to the documents the kinds wrote as
  variants.
- **`RecognizedExtension::Market(&'static MarketDescriptor)`** beside
  HEAD's `Code(DataType)` and a `State` of its own: the identity an Arrow
  field declares is the kind, and the cast plan reads `kind.member(code)`
  where it read the enum leaf.
- **`Serie::Market(MarketSerie)` holds the storage leaf inline** - sixteen
  bytes, so the 40-byte pin holds - and forwards every column verb to that
  leaf, which already reads its cells as the kind's members through the
  reading the landing resolved from the field; `column!` and `column_mut!`
  dispatch to the inner leaf, and `as_uint8`/`as_uint16` still narrow to it.
- **A kind's marker hashes nothing**, as the unit markers the kinds were
  did, and `Hash for DataType` writes the kind's `shape` where the variant's
  discriminant stood: a field of a kind therefore hashes its name, its
  nullability and its metadata alone, which is what keeps the FIX
  dictionary's pinned hash (`14_542_711_836_201_211_247`) byte-identical.
- **One enum rule**: `State` and the market kinds refuse a value of another
  enum or code kind as a value of another vocabulary (HEAD's `State` arm
  read a code's text as a spelling); a market enum's `Value::from_scalar`
  is a binary search over `ALL`.
- **The unknown-word refusal of the datatype grammar is positioned**: an
  `Error::Parse` at the word's byte offset whose reason is the one
  `unregistered` sentence (`unknown datatype "x": no registered datatype
  answers it; install the crate that claims it and call its install()`), so
  `rust/tests/root/parser.rs`'s offset pin holds; and the expression grammar
  (`typed_literal`) tries a word as a datatype only where a text literal,
  `null` or a parameter list follows it, so a bare column name costs no
  speculative parse - the one cost pin S1 moves on purpose: `DECLARED_READ`
  28 -> 21 and `ORDER_PARSE` 19 -> 12 in `allocations.rs`, seven
  allocations fewer per key spelled `count desc`, the three `sort_by` rows
  with them, each re-pinned with that sentence.
- **The logical names are a register** (`vocabulary.rs`, D8): `logical_names()`
  answers name order, `register_logical_name` claims one for a crate,
  `folded_logical_name` falls back to `kind_named`; `DataType::LOGICAL_NAMES`
  is gone, `DataType::CODES` stays HEAD's.
- **`Shape` states every position** (`#[repr(isize)]`, explicit
  discriminants) so the positions the enum kinds held stay theirs and the
  codes' are their variants' own.
- **What the second review changed** (an `opus` reviewer read the narrowed
  diff; nine findings, each taken or answered): the text-to-enum ingest
  read a spelling through the kind's `read` alone and certified the
  landing, so a kind whose reader answers a non-member code would have
  landed it - every spelling and every code now enter through
  `kind.scalar`/`kind.member` (the crate-private `adopt_spelling` and
  `adopt_member` where a validated field is in hand, so a cell pays the
  reader and the member search alone), pinned by a claimed lying kind cast
  from text; the grammar's refusal is positioned again and `typed_literal`
  guards its speculative parse, as the bullet above states;
  the public doors that mint a value (`scalar`, `member`,
  `default_scalar`) check the descriptor is the claimed one, and `claim`
  refuses the core's own name, so neither an unclaimed descriptor nor a
  crate claiming as `yggdryl` can state a core rank; `read_enum_spelling`
  and `read_enum_code` take the `DataType`, so a FIX message's and an
  Iceberg row's enum values read no register per value; `BY_BYTE` is
  written first so a name a reader answers validates already; the logical
  names claim under the market register's lock; `MarketScalar` is `Copy`;
  a safe cast of a non-member code under a nullable field lands null, as
  HEAD pins for `side`, and the harness pins that beside the strict
  refusal. Answered and left: a scalar document whose `value` precedes its
  `type` and is no text refuses with serde's untagged message, and
  `DataType`'s and `Scalar`'s serde readers stay two shapes.
- **The test-only kinds**: `rust/tests/market_register.rs` claims `testenum`
  at `0xc7` (`Code8`) and `testwide` at `0xc8` (`Code16`) as
  `yggdryl-tests`, and pins the claim, every refusal, every intake door,
  the pointer identity, the reader check, the late claim and the install
  rule; `rust/tests/root/market.rs` pins the core's four in byte order, the
  refusal of an unclaimed name, byte and tag beside the lossless Arrow
  fallback, that a code is no kind, and holds the cross-kind order pin S0
  wrote in `scalar.rs`; `rust/tests/root/plugin.rs` pins the register;
  `allocations.rs` gains the two rows the prompt names, over a `side`
  column.
- **Re-pins, each accounted for**: `logical_names()` answers name order,
  so a datatype's names list sorted (`rust/tests/root/mic.rs`,
  `vocabulary.rs`); `DataTypeId::all().last()` is `TimeInForce::ID`
  (`version.rs`); the unknown-tag refusal reads as above
  (`node/tests/datatype.test.js`, `docs/types/text/string.md`). No byte,
  rank, shape, hash feed, serde document or value-stream byte moved; the
  one cost pin that moved is the grammar's, above, on purpose.

### S1 results

Every command below ran on the tree committed as S1, from `/home/user/yggdryl`,
with `CARGO_INCREMENTAL=0` and the debug info off; a background chain holds the
cargo lock for the long steps and the foreground ran the rest.

| Check | Command | Result |
| --- | --- | --- |
| it builds | `cargo check --workspace --all-targets --all-features --keep-going --message-format=short` | clean: 0 errors, 0 warnings, the four crates |
| the register | `cargo test -p yggdryl --test market_register` | 7 passed |
| the root files | `cargo test -p yggdryl --test root` | 1681 passed |
| the series | `cargo test -p yggdryl --test serie` | 338 passed |
| the value contracts | `cargo test -p yggdryl --test value` | 25 passed |
| the digests | `cargo test -p yggdryl --test xxhash` | 81 passed |
| the dictionary hash | `cargo test -p yggdryl --test fix store` | 83 passed; `14_542_711_836_201_211_247` unmoved |
| the cell and ingest rows | `cargo test -p yggdryl --test allocations -- leaf_cell registered prebuilt` | 4 passed |
| the call counts | `cargo test -p yggdryl --test iobase_calls` | 37 passed |
| the private pins | `cargo test -p yggdryl --features internals --test root` | 1852 passed |
| the whole run | `cargo test -p yggdryl --all-targets --all-features --no-fail-fast` | 70 targets; 8968 passed in the 69 green ones; `allocations` 179 passed, 5 failed - `DECLARED_READ`, `ORDER_PARSE` and the three `sort_by` rows, each seven allocations under its pin, the grammar's deliberate move - re-pinned with that sentence |
| the re-pins | `cargo test -p yggdryl --test allocations -- declared sort_by`; `--test root -- vocabulary market`; `--test market_register`; `--doc vocabulary` | 8, 87, 7 and 3 passed |
| the code files | `git diff 6d71a36ee --stat -- rust/src/code.rs rust/src/{country,ccy,mic,cfi,isin,cusip,sedol,bbg,ric,figi,unit,forex,lei,bic,elf,dti,fisn}.rs docs/types/codes` | empty: byte-identical to HEAD (D25) |
| clippy, all features | `cargo clippy --workspace --all-targets --all-features --no-deps -- -D warnings` | exit 0 |
| clippy, default features | `cargo clippy -p yggdryl --all-targets --no-deps -- -D warnings` | exit 0 |
| the CLI | `cargo test -p yggdryl-cli --all-targets --no-fail-fast` | 35 passed, 6 ignored over its five harnesses (`fix`, `market`, `quality`, `style`, `xmla`) |
| the whole run, default features | `cargo test -p yggdryl --all-targets --no-fail-fast` | 70 targets, 6505 passed, 0 failed - after the first attempt died at the linker on a full disk (the sandbox allowance, not the tree): 20 GiB of stale `target/debug` artifacts of the workspace crates removed by `cargo clean -p`, then the run whole |
| rustdoc examples | `cargo test -p yggdryl --doc` | 626 passed |
| the API pages | `RUSTDOCFLAGS="-D warnings" cargo doc -p yggdryl --no-deps --all-features` | exit 0 |
| Python | `VIRTUAL_ENV=python/.venv python/.venv/bin/python -m maturin develop -m python/Cargo.toml`, then `-m pytest python/tests -q` and `-m mypy --strict --config-file python/pyproject.toml python/yggdryl python/tests/typing_bindings.py python/tests/typing_fields.py` | the extension installed; 2780 passed, 4 skipped - the same four as at P0, P1 and S0; mypy exit 0 |
| Node | `npm run --prefix node build:debug`; `cargo build --locked -p yggdryl-cli`; `npm test --prefix node`; `npx tsc --noEmit` in `node/`; `git diff --stat -- node/index.js node/index.d.ts` | the addon built; the CLI built; 1122 tests, 1120 passed, the 2 failing ones the sandbox `TextDecoder` pair below; tsc exit 0; the generated loader and declarations unchanged |
| the docs manifests | `node scripts/build_docs_fix.js --check`; `node scripts/build_docs_playground.js --check` | both current |
| the page examples | `python scripts/check_docs_examples.py --lang rust`, `--lang python`, `--lang javascript` | Rust 936 passed; Python 836 run, 3 skipped, 0 failed; JavaScript 788 run, 2 skipped, 0 failed |
| the site | `python -m mkdocs build --strict --config-file mkdocs.yml` | built in 18 s, no warning |
| formatting | `cargo fmt --all -- --check`; `git diff --check` | both clean |
| the inventories | `python scripts/check_api_inventory.py`; `python scripts/generate_internals.py --check` | current (181 source files and 595 `pub` names not described yet, as before); `yggdryl::internals` current |
| no test code under `src/` | `grep -rn '#\[cfg(test)\]\|#\[test\]' rust/src python/src node/src cli/src` | empty |
| CI | the run on `ccr-0fe6f9d0-ruymat` for PR #209 | read to the end after the push, before the handoff commit; the result is the `Checks` section of `.handoff/next/MARKET_SPLIT_NEXT.md` |

Not run, as the slice made nothing of theirs stale: the charset table and
interop checks, the ISIN seed check, the country and MIC table checks, every
`cargo bench` and `npm run bench:*`, the Python boundary benchmarks, the
scale run and the free-threaded lane. The two Node charset tests that compare
the package to the runtime's `TextDecoder` (`node/tests/charset.test.js`:
`decoding agrees with TextDecoder over the same names`, `iso-8859-1 is not a
spelling of windows-1252 here`) fail in this sandbox alone, whose Node 22.22
decodes the C1 range of `windows-1252` as ISO 8859-1 does; they failed at P0,
P1 and S0 the same way and CI's Node proves them.

## S1: file sets per worker

The S1 sweep is partitioned by path so no two workers touch one file; each
worker returns its script and the files it edited, and one
`cargo check --workspace --all-targets --all-features --keep-going
--message-format=short` after the last worker is the next list.

| worker | files |
| --- | --- |
| A (design, foreground) | `rust/src/plugin.rs`, `rust/src/market.rs`, `rust/src/datatype_id.rs`, `rust/src/datatype_kind.rs`, `rust/src/lib.rs` |
| B | `rust/src/datatype.rs`, `rust/src/default.rs`, `rust/src/parser.rs`, `rust/src/vocabulary.rs`, `rust/src/compatibility.rs` |
| C | `rust/src/scalar.rs`, `rust/src/arithmetic.rs`, `rust/src/valuestream.rs`, `rust/src/variant.rs`, `rust/src/xxhash/scalar.rs` |
| D | `rust/src/serie.rs`, `rust/src/serie/value.rs`, `rust/src/serie/order.rs`, `rust/src/budget.rs`, `rust/src/media/inference.rs` |
| E | `rust/src/serde.rs`, `rust/src/field.rs`, `rust/src/typed.rs`, `rust/src/value/mod.rs`, `rust/src/value/canonical.rs`, `rust/src/arrow/extension.rs` |
| F | `rust/src/cast.rs`, `rust/src/string.rs`, `rust/src/enums.rs`, `rust/src/code.rs`, the 21 kind files, `rust/src/state.rs`, `rust/src/ascii.rs` (its doctest at `:253`), `rust/src/boolean.rs` (`:167`) |
| G | the small dispatch sites: `rust/src/{json,toml,xml}/wire.rs`, `rust/src/yaml/mod.rs`, `rust/src/text/mod.rs`, `rust/src/expression/display.rs`, `rust/src/merge.rs`, `rust/src/graph/mod.rs`, `rust/src/fix/*.rs` sites the compiler lists |
| H | `python/src/**`, `python/yggdryl/**` |
| I | `node/src/**` |
| J | `rust/tests/**` re-spelled from the compiler's list (never the S0 pins' literals) |
| K | `docs/**`, `skills/**`, `.api-inventory.txt`, `AGENTS.md` |
| L | `rust/benchmarks/**` (`arrow.rs:735, 847, 980`, `types/datatype/{ascii,serie,value}.rs`, `types/field/value.rs`) and `cli/**` (`cli/tests/fix.rs:1118`) |

The compiler's list misses what `cargo check` never compiles: the 42
doc-comment lines in 29 `rust/src` files that name a deleted variant are
driven from `cargo test -p yggdryl --doc` and an `rg` over `///` lines, and
belong to the worker whose files hold them.
## The ledger

| D# | decision | evidence | slice |
| --- | --- | --- | --- |
| D1 | program branch `ccr-0fe6f9d0-ruymat` (harness-assigned); draft PR; merge `origin/main` per session; version `0.1.21` until S9 | the harness's branch rule; `ci.yml:3-9` | S0 |
| D2 | closed shape over `&'static MarketKind` descriptors; `Scalar` 48 and `Serie` 40 bytes | the spike; `allocations.rs:6540` | S0, built S1 |
| D3 | `DataTypeId(u8)` newtype with CamelCase consts; the four enum kinds' ids leave it and the seventeen codes' stay (D25); `all()` answers the claimed order | the spike; `datatype_id.rs:28-32, 270` | S0, built S1 |
| D4 | `State` and the event vocabulary stay core; `State`'s FIX tables move to `fix/` in S3 | `text/line.rs:11-12`, `text/plan.rs:20` | S0 |
| D5 | the market type's own `dtype()`/`field(name)` items; no extension trait | E0116 | S3 |
| D6 | one public `implementer` module of forwarders, per-slice list from a scratch `git mv` | AGENTS "never make an item pub inside a published module" | S1, S3, S6 |
| D7 | explicit idempotent `install()`; intake alone reads the register; the refusal list; the core seeds itself until S4 | `lib.rs:11` denies `unsafe`; `datatype_kind.rs:140-178` | S0, built S1 |
| D8 | `plugin.rs`: one claim-once `Register<K, V>` and the `Dyn*` helper; market, media and `LOGICAL_NAMES` keys on it | `expression/user.rs:483-494` | S0, built S1/S2 |
| D9 | a market trait for a message that splits; `MarketKind::Fix` stays; a protocol-view builder | `graph/market_data.rs:44`, `protocol.rs:2078` | S3 |
| D10 | the table above | `scheme.rs:27-420`, `mime_type/line.rs:80-100`, `logging/facade.rs:72-76`, the 59 `warned!` and about 120 `log` sites counted there | S3 |
| D11 | option (d); sizes recorded; namespace proven in S5; FIX inside the market packages | pyo3 `type_object.rs:88-89`; napi `type_tag.rs:7-15`; the registry reads above | S0, built S5 |
| D12 | the five crate names, `yggdryl-market` on PyPI and npm (npm put to the user) | the availability reads above | S0, published never by a session |
| D13 | one `AGENTS.md`, one site, docs runner into `cli/tests/docs_examples.rs` | `check_docs_examples.py:46, 207-213`, `.gitignore:40` | S4 |
| D14 | a test-only kind re-fixtures the core; `ulbridge.log` to `support/` | the 90-line grep | S4 |
| D15 | `s3tables` a feature of `yggdryl-iceberg` | `iceberg/table.rs` location doors | S6 |
| D16 | no Avro in the core; `yggdryl-avro` carries `snap` | `rust/Cargo.toml` features | S6 |
| D17 | the core keeps the Iceberg type-string spelling | `lib.rs:79-81` | S2 confirms |
| D18 | deferred to S8a | the prompt's S8a section | S8a |
| D19 | nothing moves: ranks, `Shape` positions and feeds as listed | the pins; `scalar.rs:1202`, `datatype.rs:749-844` | S0, held S1 |
| D20 | `MarketValue` with an owned `from_scalar`; `Value` unchanged; the kind's ZST marker over `MarketType` | `value/mod.rs:78-90` | S0, built S1 |
| D21 | one trait per role on the register; the pushdown rides the options | `media/options.rs:142, 1343`; `iceberg/mod.rs:146-318` | S0, built S2 |
| D22 | `pluginside` deleted; the plugin's role is a `Side` read by `fix::plugin_side`; `buyside`/`sellside` spell `BUYS`/`SELL`; byte `0xc6`, `Shape` 72 and rank 32 retired; the dump, the codeset and the dictionary hash moved with it | the user's instruction; `side.rs`, `fix/source.rs`, `fix/crated.rs` | P0 |
| D23 | the medium holds its `RecordOptions`; the media serie keeps no copy | the user's instruction; `media_serie.rs:18`; the understanding workflow's map | P1 |
| D24 | `yggdryl-excel`, a seventh crate through the media point, claimed in S2, its own commit after S6; the bindings keep their Excel doors in the one native module | the user's instruction; the 39 core sites and the import lines above; crates.io 404 | S0, built S6b |
| D25 | the seventeen codes stay core and flat; the register holds enum kinds alone (`Code8`/`Code16`), `MarketPayload`, `is_canonical`, `respell` and `CODE_VALUE_RANK` deleted; `yggdryl-market` carries the enums, `graph/` and the ISIN registry | the user's instruction; the S0 pins; one free Code byte | S1 |

## Review (S0)

One reviewer (an `opus` subagent) read this file against AGENTS.md and the
baseline counts, re-ran nine of the recorded commands verbatim at
`2ae975674` (every count matched) and at HEAD (every move the PluginSide
deletion's), and returned fifteen findings. Each was taken:

1. `DataType`'s order is a third per-kind table, `dtype_rank`, caller-visible
   through both bindings - D2 and D19 now carry it, and S0 pins it.
2. `MarketValue` named both D2's payload enum and D20's trait - the enum is
   `MarketPayload`.
3. D20's supertraits: `CodeValue: Value` cannot hold for a code whose value
   is a `Str` inside `Scalar::Market` - `CodeValue` requires `MarketValue`
   instead; the enums keep `Value` through their static member table.
4. `MarketKind` named both the descriptor and `graph::MarketKind` - the
   descriptor is `MarketDescriptor`.
5. D21's registered options would have moved `RecordOptions::stable_hash`,
   a persisted feed - the registered form hashes today's tag and struct in
   today's field order, and S2 pins every medium's value first.
6. D6 could not carry a crate-private type or a macro by forwarding alone -
   the mechanism is stated item by item (`InstantSequence` defined in
   `implementer.rs`; the four macros `#[macro_export]` expanding through
   `$crate::implementer`).
7. D10's logging row missed `warned!` and its 59 sites - the facade maps
   `yggdryl_<crate>::` to `yggdryl.` in one place, pinned in
   `rust/tests/logging/`.
8. D23 and the P0 and P1 rows were missing - added; the pins are generated
   and green on P1's tree; P0's count moves are recorded.
9. The listing pin could not survive S1 through `DataTypeId::ALL` - it reads
   every byte through `from_u8`, every pin builds values through surviving
   doors, and the `ALL` length and zip are S1's one allowed re-pin.
10. S1's worker sets left benches, `cli/`, two doctests and 42 doc-comment
    lines unowned - worker L and the `--doc` drive are added.
11. D24's evidence: `Str` is public and Excel reaches about 25 crate-private
    items by full path - corrected, the exact list S6b's.
12. Excel must be claimed in S2 so S6b is a move only; the crate name was
    never checked - claimed in S2; crates.io answers 404.
13. The baseline table was not reproducible - the `\\|` escape is stated,
    15 is 17, the inventory regex is spelled.
14. D7's "names the crate to add" needed a second listing, and the code
    range has one spare byte - the lookup refusal is generic, a claim
    refusal names the first claimant, D7 reads `DataTypeKind`, the capacity
    is a risk.
15. The stubs cannot split (D11), 21 not 18 `impl DataType` blocks (D5), the
    listings need a register key and `LOGICAL_NAMES` becomes
    `logical_names()` (D8, D10), no medium routes the FIX MIME types (D10),
    the two Parquet statistics readers move to `yggdryl-parquet` (D21), and
    the ledger's missing evidence (D10, D13, D18) - each written in.

Found sound and left as written: D3's `all()` order, D4's boundary, D11's
citations and pickles, D13, D15, D16, D17, D19's positions and ranks and the
new dictionary pin, D24's counts; no test code under any `src/`; P0's Node
`fix.pluginSide` re-spells an existing door and adds none.
