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
`yggdryl_market::graph` re-exports none of it. `State`'s FIX doors -
`from_fix_status`, `FIX_STATUS_TAGS` and `from_fix_msgtype` - move to
`fix/state.rs` as the free functions `from_status`, `STATUS_TAGS` and
`from_msgtype` in S3 and to `yggdryl-fix` in S4, with the Python classmethods
redirecting in S3 and moving to `yggdryl.market.fix` in S5, no alias. The 17
codes stay core and flat (D25); they are not market.

### D5, constructor spelling (S3)

Decided: the market type's own associated items - `Side::dtype()`,
`Side::field(name)`, written once in `enum_leaf!`'s market arm for the four
kinds - replace the four `impl DataType` blocks the kind files hold
(`side.rs:293`, `timeinforce.rs:141`, `marketdatakind.rs:279`,
`marketdatatype.rs:547`); no extension trait preserves `DataType::side()`.
The eighteen code blocks (`DataType::isin()` and its siblings) stay (D25),
and so do `DataType::is_code`, `code_width` and `code_name`, in the core
reading the descriptor. S1 leaves the constructors where they are, S3 applies
the spelling in place.

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
where it sits is what AGENTS forbids; a macro a moving crate invokes
(`warned!` at `logging/warning.rs:107-121`, `enum_leaf!`,
`define_field_types!` at `typed.rs`, whose users are the four market kind
files, `bytes_dtypes!`, `bytes_scalars!` and `string_scalars!`, and D9's
`protocol_field_types!`) is `#[macro_export] #[doc(hidden)]`, re-exported
from `implementer`, and every path it expands to is spelled
`$crate::implementer::...` so the expansion names nothing private -
`warned!` expanding to `$crate::implementer::warn`. `media_serie!` has no
user outside the core after S2b and `delegate_event!` (`graph/mod.rs:47`) is
market-internal, so neither is exported; S4 respells market's `delegate_*`
macros.
Refused: a feature flag that raises `pub(crate)` items (the same API change
wearing a feature's name, AGENTS); a second copy of a type per crate. S1 publishes what S1 needs (the register's claim doors and nothing
private), S3 the market and FIX list, S6 the media list. S3's is proven
without the new crates, by re-pointing every leaving-file site to
`crate::implementer::X`, a clean `cargo check --workspace --all-targets
--all-features --keep-going` and no non-`pub` cross-crate path left; S4's
`cargo check -p <crate> --message-format=short` finds the method-reached
residue, and S6's list is derived from a scratch `git mv` plus the same.

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
| FIX Latest names in `LOGICAL_NAMES` | the 32 FIX datatype names move to a fix-owned table (`fix/mod.rs`, `pub(crate)`) that the core's seed claims on D8's register until S4, when `yggdryl-fix` registers them itself; the core keeps `mic`, `exchange`, the code names (D25: the codes stay core) and `state`. S3 first greps every reader of a FIX word outside `fix/` |
| `DateTime64::from_fix_text`/`from_fix_clock`, `Time32/Time64::from_fix_text`, the grammar in `temporal.rs:844-914` | stay in the core, `pub(crate)`, and reach `yggdryl-fix` through `implementer` forwarders: the premise that the ISO part delegates to the public readers is false (it is one flag-selected `clock_at` body over private helpers) and making the readers public would create API; reason recorded by S3 |
| the FIX entry of `for_each_well_known_protocol!`; the `FIX:*` keys | move with the view (D9); `Field::as_fix`, `Field::as_fix_mut` and `Metadata::as_fix` are deleted with the entry |
| `Scheme::FIX` | stays: `metadata.rs`'s stack-key read serves known schemes only, and a custom scheme would allocate per read and break `allocations.rs:789` (S3 read `scheme.rs:27, 88, 169, 210, 420`) |
| `MimeType::{FIX, FIXUL, FIXML, ULLINK}`, the frame classifier | stay as routing vocabulary and the one bounded content read; no medium routes them (`Media::open_as` has no arm; only `mime_type/line.rs` and the bindings' enum tables name them), so nothing registers for them |
| `parallel.rs` | stays, generic; `ordered` reached through D6; its AGENTS row restated without FIX |
| logging targets | the facade names a logger by its module path under `yggdryl` whatever crate holds the module, through a per-crate table in one place (`logging/facade.rs` `is_foreign`/`logger_for`: `yggdryl` and `yggdryl_market` log under `yggdryl`, `yggdryl_<folder>` under `yggdryl.<folder>`, so `yggdryl_fix::build` is `yggdryl.fix.build`, where the literal rule `yggdryl_<crate>::` to `yggdryl.` first written here would have logged `yggdryl.build`; `yggdryl_cli` stays foreign; the old `strip_prefix("yggdryl")` would otherwise leave `_fix::` and file every moved record as foreign), so the 60 `warned!` sites (58 leave the core; 28 in `fix/market.rs`) - which pass `module_path!()` as target and deduplication key - and about 120 `log` sites (`fix/` 81, `iceberg/` 29, `graph/` 4, the ISIN registry 6) keep every logger name byte-identical with no edit; the deduplication key is the mapped name; pinned in `rust/tests/logging/` with a record from a `yggdryl_fix::` module. Refused: an explicit `target:` on every site (about 180 edits, and `warned!` has no target argument) |
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
- CI time: each new crate is one line under `[leaves]` in
  `.github/ci/rows.toml`, uncommented in the change that creates its
  manifest: its suite runs as `Leaf` jobs in both feature lanes, at once,
  and a change under its folder runs what proves it alone ("The CI
  structure" below). A target that moves out of the core takes its shard
  of `[shards.yggdryl]` with it, and an exchange that moves into a leaf is
  named in its line's `jobs`.

## The slice table

| S | commit | proved by |
| --- | --- | --- |
| P0 | `pluginside` deleted, the plugin's role a `Side` (U8, D22): "Delete PluginSide; the plugin's role is a Side" | the whole run in the all-features lane; both bindings' suites; the three example passes; CI read |
| P1 | the medium holds its `RecordOptions` (U9, D23) | the media suites and their `iobase_calls` rows; both bindings; CI read |
| S0 | design, the pins, the prompt and this file: "Pin the market kinds' wire and cost contracts" | pins generated and green on P1's tree; pushed; CI read green |
| S1 | the market extension point; the four enum kinds claimed in place, the seventeen codes the core's flat variants (D25) | S0's pins byte-identical; the cost and size gates; `rust/tests/market_register.rs` |
| S2 | the media extension point (D21), the media claimed in place | `iobase_calls` unmoved; every medium's harness; both lanes; the exchange jobs |
| P2 | the medium holds its origin and its cache under `cache_ttl`, the serie composes the pushdown (D34, D35): "Hold the origin field and its cache on the medium; compose the pushdown once" | the `s2_pins` byte-identical; every cost row unmoved or down; the new TTL, write-update, projection and composer pins; both bindings; CI read |
| S2b | the XMLA medium registered in place (D33): "Register the XMLA medium in place" | the three `xmla` hashes and the order pin of `mod s2_pins` byte-identical; `rust/tests/xmla`; `media_register`; the `xmla` rows of `iobase_calls` and `allocations` unmoved |
| S3 | the remaining seams in place: D5, D6, D9, D10 | `cargo check --workspace --all-targets`; the dictionary hash and crate dump unmoved; the whole run |
| S4 | `yggdryl-market` and `yggdryl-fix` | market and fix whole runs in both lanes; cost rows unmoved; `cargo package --list`; maturin sdist |
| S5 | the market binding packages; release and CI learn them | the packages' suites in CI; a rehearsal on the user's go |
| S6 | `yggdryl-avro`, `yggdryl-parquet`, `yggdryl-iceberg` with `s3tables` | each crate's suite; the exchanges; the Parquet `iobase_calls` rows; the MSRV job |
| S6b | `yggdryl-excel` | its suite in both lanes; the Excel exchange job (openpyxl); `rust/tests/interop/excel.rs` no longer skipping |
| S6c | `yggdryl-xmla`, with `soap/` | its suite in both lanes; the `xmla` rows of `iobase_calls` and `allocations` unmoved; `yggdryl xmla serve` built against it; the XMLA page's examples |
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
## S2: design

The media extension point, D21 built, with S1's mechanism: a medium states
what it is once, in its own file, as a `static` implementing one role trait,
and claims it on `plugin::Register` under a key its intake reads once; past
intake a value in hand carries its medium and nothing reads the register per
leaf, per batch or per row. The core claims its own media before a register
answers anything (`seed()`, as `market::seed()`), until S6 replaces each
seed with the leaving crate's `install()`. Four roles, four registers, one
refusal shape: the pinned phrase of the door that refuses today, then
`install the crate that claims it and call its `install()``. Every figure
below was counted at `719299cf6` by the seven maps under the scratchpad's
`s2_map/`; the S2 results table re-counts them at the slice's HEAD.

### D26, the registered options

Decided: `RecordOptions::Registered(RegisteredOptions)` beside the core
media's variants `Ipc`, `Text`, `Xmla`, `Csv`, which never leave (D33 amends this: `Xmla` leaves in S2b, and the core's own are `Ipc`, `Text`, `Csv`);
`Parquet`, `Avro` and `Excel` are deleted, so the three leaving media are
registered in place and S6 moves files. `RegisteredOptions` is a newtype
over `Box<dyn MediumOptions>`. `MediumOptions` is the object-safe twin of
`IORecordOptions`: the fourteen shared-section pairs (`declared`, `name`,
`safe`, `batch_row_size`, `batch_byte_size`, `max_row_size`, `row_offset`,
`max_byte_size`, `commit_batch_num`, `num_threads`, `level`, `merge_by`,
`filter`, `select`, each with its setter), what a trait object owes a
derive-heavy enum (`clone_box`, `dyn_eq`, `dyn_cmp`, `dyn_hash`, `as_any`,
`as_any_mut`), the medium (`codec() -> &'static dyn MediaCodec`,
`mime_type()`), `stable_hash()` and the two hooks a core door shares across
media - `header`/`set_header` (CSV and Excel; Python's and Node's one
`header` property) and `file_threads`/`set_file_threads` (Parquet and Avro;
what a table hands each file). One blanket impl over `T: IORecordOptions +
MediumSettings + Clone + Eq + Ord + Hash + Debug + Send + Sync + 'static`
writes it, so a medium adds `MediumSettings` - `const CODEC`, and the two
hooks with defaults - and nothing else; the core's four options implement
it too, so one typed door serves every medium.

The settings struct is today's whole options struct, shared fields included,
held behind the box: `RecordOptions::stable_hash` for a registered medium is
`stable_hash_of(&(codec.name(), settings))`, byte for byte today's variant
arm, which the S2 pins in `rust/tests/media/options.rs` (`mod s2_pins`)
prove - the hash feed is persisted (D19), so the shared sections are never
split out of the struct. The enum's own `Ord` and `Hash` are hand-written by
the codec's `rank()`, today's declaration position - `ipc` 0, `parquet` 1,
`avro` 2, `text` 3, `xmla` 4, `csv` 5, `excel` 6, a medium outside the core
at or above `EXTERNAL_RANK` (32) - so `arrow.stream < parquet < avro <
text/plain < xmla < tsv < csv < xlsx` holds as the pin states it, and a
registered pair of one rank orders by `dyn_cmp`. `Eq` reads the type and
the value (`dyn_eq`).

The typed door: `settings::<T>()` / `settings_mut::<T>()` (a downcast on
any variant's payload; `None` on another medium) and
`require_settings_mut::<T>(path, setting)`, which refuses with today's
pinned sentence `expected {title} options to set {setting}, got {mime}
options` from `MediaCodec::title()` - `Parquet`, `Avro`, `Excel`, `CSV`,
`text` - at `path`; `From<T> for RecordOptions` wraps any `MediumSettings`
struct. `RecordOptions::parquet_*`, `avro_*`, `excel_*`, `push_parquet_key_value`
and `set_file_threads`' arms are deleted: their bodies are the structs' own
methods (`ParquetOptions::compression_name`, `set_compression_name`,
`set_max_row_group_size`, `set_key_value_metadata`, `push_key_value` exist;
`AvroOptions::block_codec`/`set_block_codec`/`sync_marker`/`set_sync_marker`
and `ExcelOptions::sheet`/`set_sheet`/`range`/`set_range` are written from
the deleted bodies), reached by the bindings' properties, which keep their
names, through the typed door. The CSV accessors and `timezone` stay on
`RecordOptions`, their `_ =>` arms refusing every other medium. `mime_type`
and `header` lose `const`. Refused: `Arc` over the box (a set clones the
struct either way); a registered variant carrying the fourteen sections
beside a settings object (D21's first wording), because the hash feeds the
whole struct in declaration order.

### D27, the codec and the wrapper

Decided: `MediaCodec` in `media/codec.rs`, object-safe, `Debug + Send + Sync
+ 'static`, one `static` per medium in the medium's own file (`IPC_CODEC`,
`PARQUET_CODEC`, `AVRO_CODEC`, `TEXT_CODEC`, `XMLA_CODEC`, `CSV_CODEC`,
`EXCEL_CODEC`), the core's four and the leaving three alike:

| method | replaces |
| --- | --- |
| `name()` (the hash tag, `"parquet"`), `title()` (`"Parquet"`), `rank()`, `mime_types()` (`ARROW_STREAM` and `ARROW_FILE` for IPC, `CSV` and `TSV` for CSV; the first canonical) | the four MIME chains of `for_mime_type`, `mime_type()`, `Media::open_as` and `Holder::into_media_base` |
| `default_options(base) -> RecordOptions` | `RecordOptions::for_mime_type`'s arms (CSV picks the TSV dialect by `base`) |
| `compresses_internally()` - `true` for Parquet and the workbook, `false` for Avro, so `.avro.gz` composes as today | the two outer-coding declines in `holder/mod.rs:747-758` |
| `has_row_identity()` - `false` for text lines | `merge_leaf`'s text refusal |
| `read_batch_reader(handle: &dyn IOBase, declared, options)`, `row_size`, `read_field` (Parquet's clears the root's metadata), `overwrite_arrow_reader(handle: &mut dyn IOBase, ..)` | `leaf_reader`, `leaf_row_size`, `leaf_field`, `leaf_writer` (`transfer.rs:1171-1290`) |
| `stated_field(handle, options) -> Option<Field>` (default: the probe under the medium's default options and the options' name) | `stored_field`'s Text, Xmla, Csv and Excel arms |
| `read_stream(handle, declared, options) -> Option<StreamSerie>` (default `None`; Csv, Text and Avro answer their native streams) | `read_record_serie`'s arms (`iomedia.rs:1488-1494`) |
| `open(handle: Holder) -> Media` | `Media::open_as`, `Media::{parquet, avro, excel}`, the `From<Parquet<Holder>>` impls |

`RecordOptions::codec()` is the one dispatcher past intake - a core variant
its medium's static, the registered box its own - and `transfer.rs`,
`iomedia.rs` and `media/mod.rs` name no medium: each former match is one
call through it. Text's and CSV's core rules (`TextLeaf`, the CSV header
target, the container scan's text-last rule, plain text's `Holder::Text`
rather than a `Media`) stay on the core variants where they are. The
register: `media::codec::claim(codec, by)` claims every MIME type the codec
names, under one lock, all or none, refusing a type claimed already (naming
the first claimant), the core's own name as `by`, a codec naming no type,
and a rank below `EXTERNAL_RANK` from outside the core; `codec_for(base)`
is the one intake lookup, its refusal `expected a record encoding this build
implements (<every claimed type in rank order>; install the crate that
claims it and call its `install()`), got <base>` - the phrase Node and the
skill pin, the ORC and `.xls` texts the Rust and Python tests pin, with the
`parquet` feature note gone because the list is what is claimed - and
`for_mime_type` keeps its structured-document sentence after a failed
lookup; `codecs()` lists the claims in rank order; `seed()` claims the
core's seven once. `Media::open_as` is `codec_for(base)?.open(handle)` and
answers the same sentence as an `arrow::Error`.

`Media::Registered(Box<dyn MediaWrapper>)` beside `Ipc`, `Text`, `Xmla`,
`Csv`; `MediaWrapper: IOBase + Debug` adds `codec()`, `handle()`,
`into_handle(self: Box<Self>)` and `with_field(self: Box<Self>, field)`;
`Parquet<Holder>`, `Avro<Holder>` and `Excel<Holder>` implement it, and the
`Media` match methods gain one arm. `Media::medium()` names the medium, so
Python picks its handle class (`Parquet`, `Avro`, `Excel`) by the codec's
name and asks the store nothing; `handle()` loses `const`.
`Holder::into_media_base` composes through `codec_for`, composing nothing
for an unregistered type (today's `supported` test) and declining the outer
coding where `compresses_internally()` says so.

### D28, the serie

Decided: `Serie::Parquet`, `Serie::Avro`, `Serie::Excel` and
`Serie::IcebergTable` are deleted with `ParquetSerie`, `AvroSerie`,
`ExcelSerie` and `IcebergTableSerie` - each `media_serie!` invocation passed
no native read, so each was `GenericMediaSerie` under another name; a
leaving medium is read as `GenericMediaSerie::new(media)` and
`Serie::GenericMedia`. `require_kind` takes the MIME types a leaf accepts
(`media_serie!` gains `accepts = ...`: IPC its two, CSV its two, Text and
XMLA one, `WarehouseTable`, `Http` and `GenericMedia` every type) in place
of the variant-name match, its refusal unchanged; the macro's unused
`$read` arm goes. Nothing outside `rust/tests/root/media_serie.rs` named a
deleted serie.

### D29, the table format

Decided: `TableFormat` in `media/format.rs` - `name()`, `locate(handle:
&dyn IOBase) -> Result<Option<Box<dyn LocatedTable>>>` - and
`LocatedTable: Debug + Send` with today's fifteen `Located` methods
(`is_whole`, `overwrite_whole`, `clear`, `stored_field`,
`read_arrow_field`, `read`, `row_size`, `column_size`, `record_options`,
`overwrite_arrow_reader`/`append_arrow_reader`/`merge_arrow_reader(batches,
options) -> IOResult`, `overwrite_prepared(batches, threads)`,
`append_prepared(batches, threads) -> u64` the rows declined,
`merge_prepared(batches, merge_by, safe, threads)`); the
`ReplacedPartitions` accumulator moves inside the located table, which a
write session locates once and holds for its life, so no Iceberg type
crosses the trait. `media::format::locate(handle)` asks every claimed
format in name order, answering `None` before touching the handle where
nothing is claimed, and is what the eleven `crate::iceberg::located` sites
call (`isin_registry/store.rs` 2, `iobase/hierarchy.rs` 1, `transfer.rs`
4, `iomedia.rs` 4); `ArrowWriteTarget::Iceberg` becomes `Table { located,
stored }`, `Opened::Table` holds the box, `WriteCount::skip` loses its
gate. `Located` implements the trait in `iceberg/mod.rs`, `ICEBERG_FORMAT`
is the static, claimed by the seed under `cfg(feature = "iceberg")`; the
ISIN registry store reads `locate` and the three methods it needs. The
core keeps the layout detection the folder catalog reads (one `metadata/`
listing: `version-hint.text` or `*.metadata.json`) as routing vocabulary,
the way `MimeType::PARQUET` stays (D10), and a table met with no format
claimed is refused at `$.encoding` naming the crate to install - decided
over a feature-less build reading a table's `data/` leaves as a plain
folder, which reads a table wrong instead of refusing it by name.

### D30, the warehouse

Decided: `Catalog::Registered(Box<dyn RegisteredCatalog>)`,
`Namespace::Registered(Box<dyn RegisteredNamespace>)` and
`Table::Registered(Box<dyn RegisteredTable>)` beside `Memory`/`Folder`,
`Memory`/`Folder` and `Media`; each trait the role's value contract
(`CatalogValue`, `NamespaceValue`, `TableValue`, with `IOBase`) plus
`implementation_name`, `clone_box`, `dyn_eq`, `dyn_hash`, `as_any`,
`as_any_mut`, `with_properties` and, for a table, `inheriting` and
`exists`; the Iceberg and S3 Tables catalogs, namespaces and tables land
there, the `Iceberg`/`S3Tables` variants and `Table::Iceberg` deleted with
their `From` impls, and each enum gains `downcast_ref::<T>()` /
`downcast_mut::<T>()` so a binding's `IcebergCatalog` class reaches its
value. `CatalogFactory` (`warehouse/catalog.rs`), keyed by the `type` word
(`hadoop`) and by the URL scheme (`s3tables`): `catalog(name, url, arn,
properties) -> Result<Catalog>`; `Catalog::from_url` reads the explicit
word first - the `$.with.type` refusals keep their two spellings, the
listed words computed from the claims (`memory`, `folder` and the claimed
words) - then the scheme, then `memory`/`folder` as today. `Locator`
(`holder/locator.rs`), keyed by scheme: `names(location)` (an ARN of the
service too, read before the identifier is lowered) and `holder(location,
properties)`; `Holder::from_url` asks the locators first and keeps every
byte-backend arm (`local`, `zip`, `s3`, `http`), which stay the core's; a
scheme no backend holds and no locator claims is refused naming the crate
to install. `Site::Store` is gated to `s3` alone, with a targeted
`dead_code` allowance until S6's implementer door constructs it from
outside. The folder catalog's layout detection constructs a located table
through `TableFormat` and refuses by name without a claim. The Iceberg
folder names (`metadata/`, `version-hint.text`, `data/`) stay in the core
as that detection's vocabulary.

### D31, the external error and the capability

Decided: `Error::Iceberg { reason, source }` becomes `Error::External {
origin: &'static str, reason, source }` - Display `{origin} error:
{reason}`, so an Iceberg failure prints `Iceberg error: ...` as today, the
source kept and downcastable (`is_forked`, `is_moved_base` test
`source.is::<T>()`), `is_source_failure` true - built by
`Error::external(origin, reason)` and `Error::external_with_source`;
`Error::iceberg` and `from_iceberg` stay under the gate as its two Iceberg
spellings. `From<ParquetError> for crate::arrow::Error` is deleted - an
orphan impl no leaving crate can write - and the Parquet module maps its
own crate's failures through `arrow::Error::external` at each `?`.
`IOMedia::as_any(&self) -> Option<&dyn Any>`, default `None`, replaces
`parquet_footer`: the Parquet wrapper answers its footer cache - a
non-generic object, because `Parquet<H>` cannot be downcast without naming
`H` and `IOMedia` is not `'static` - and `Holder`, `Media`, `Buffered`,
`Coded`, `Table`, `MediaTable` and the two delegating macros forward it;
`read_parquet_statistics` and `read_parquet_geospatial_statistics` leave
`IOMedia` for `parquet::read_media_statistics(&dyn IOMedia)` and
`read_media_geospatial_statistics`, carrying `parquet_leaf`'s pinned
`expected Parquet media, got ..` refusal, and every forwarder goes (nine
files and the two macros). The bindings keep their four and two methods
redirecting to them.

### The review's amendments (S2)

An independent read of the core diff, after the suites passed, changed six
things, each a defect a passing suite did not catch:

- `RecordOptions::registered` answers the core's own four structs as their
  variants, so one struct is one value whatever door built it and the enum's
  `Eq` and `Ord` agree; `RegisteredOptions::new` is crate-private.
- `media::codec::claim` refuses a second claim of a medium's `name()`, the
  tag its options hash under and what orders two media of one rank.
- `media::codec_of(base) -> Option<..>` is the lookup `Holder::into_media`
  asks, allocating nothing on a miss; `codec_for` builds the refusal only
  where it is answered.
- `parquet::read_media_statistics` and `read_media_geospatial_statistics`
  treat a Parquet wrapper as HEAD's overrides did - the footer it holds,
  refilled through `ParquetFooter::metadata` where a publication cleared it,
  at no ask of the store of what it is - and run `parquet_leaf`'s encoding and
  leaf checks for every other media alone.
- `TableFormat::table(path, root, inherited) -> Table` is the table object a
  folder catalog lists for a folder laid out as the format's table;
  `warehouse/folder.rs` asks the claimed format (`TABLE_LAYOUT_FORMAT`, the
  one layout the detection reads) and is no longer gated, and a layout no
  claim reads is a media table refusing by name.
- The merge refusal of a medium without row identity names that medium and
  lists the claimed media that have one; the native-stream door borrows the
  declared field rather than cloning it.

Recorded as decided otherwise: `MediumSettings::medium()` is an associated
function rather than D26's `const CODEC`; `MediaWrapper::medium()` and
`Media::medium()` rather than `codec()`, because `codec` is `IOBase`'s
content coding; the `From<IcebergCatalog>`-style impls stay in the
implementations' files, answering the `Registered` variants, because every
call site builds through them; `RecordOptions::require_settings` is the door
a codec reads its own struct back through; `holder::locators()` and
`media::format_named` are the registers' listing and lookup doors beside
`codecs()`/`formats()`; the ISIN store's whole-table refusal reads `expected
a table whole for the instrument registry, got a partition of one`.

### D32, pushdown, logging, D17

Decided: `filter_phases` is published (`expression::filter_phases`, eight
callers) beside the already public `Bound`, `Bounds`, `ColumnBounds` and
`Residual`; `apply_columns` keeps its signature. Logging moves nothing:
the twenty-nine `log::` sites under `yggdryl::iceberg::{staging,table}`
keep their module targets and D10's rule covers them when they move in S6
(`rust/tests/logging/facade.rs:85` pins `yggdryl_cli::shell` as foreign, so
a `yggdryl_<crate>::` mapping excludes the CLI). D17 confirmed: the default
core reaches `yggdryl::iceberg` through `iceberg/types.rs` alone, nothing
outside it reads `PrimitiveType` ungated, and `PrimitiveType::into_official`
(`cfg(iceberg)`, four callers) becomes a free function when the rest leaves.

### D33, `yggdryl-xmla` (S2b, S6c)

Decided by the user mid-S2's handoff ("Isolate also the xmla in its crate
project"). XML for Analysis is a medium like Avro, Parquet and the workbook
and leaves the same way, through D21's media point: `xmla/` is 7,040 lines
in eleven files and `soap/`, the SOAP 1.1 envelope only XMLA speaks, 995 in
one; together they name nothing of the core a medium may not - `git grep -h
'^use crate::' -- rust/src/xmla rust/src/soap | sort -u` is 26 lines:
`arrow::{BatchReader, arrow_schema_from_field, field_from_arrow_schema}`,
`expression::{Location, Plan, Source, Target}`, `holder::Holder`,
`http::{Body, Method, Request, Response, Server, Status}`,
`media::{IORecordOptions, Media, MediaCodec, RecordOptions}`, `text::Limits`,
`xml::{Element, ATTRIBUTE_PREFIX, XSD_NAMESPACE, XSI_NAMESPACE}`, the value
types and `Serie`. D26 called `Xmla` a core variant that never leaves; this
decision amends it, so the core's own media are three - Arrow IPC, plain
text, CSV - and S2b registers XMLA in place as S2 registered Excel:
`RecordOptions::Xmla`, `Media::Xmla`, `Media::xmla`, `From<Xmla<Holder>>
for Media`, `Serie::Xmla` and `XmlaSerie` are deleted - 32 lines in
`media/options.rs`, `media/options/dispatch.rs`, `media/mod.rs`, `serie.rs`
and `media_serie.rs` at `2d800d51b` - `Xmla<Holder>` implements
`MediaWrapper`, `XmlaCodec::open` answers `Media::Registered`,
`From<XmlaOptions> for RecordOptions` is `RecordOptions::registered` in
`xmla/options.rs`, and a media serie over an XMLA handle is
`GenericMediaSerie` (D28 extended). `XMLA_CODEC` keeps rank 4, its name and its one MIME type, and
the core seeds it until S6c; the three `xmla` hashes of `mod s2_pins` and
the order pin hold byte for byte, because the hash is
`stable_hash_of(&(codec.name(), settings))` for a core variant and a
registered one alike (D26). What stays core: `MimeType::XMLA` and the
`.xmla` suffix (routing vocabulary, as `PARQUET` and `XLSX` are), the
catalog `type` word `xmla` that `Catalog::from_url` refuses by name (D10),
the `xml/` codec `soap/` reads through, and `http/`, which the provider
routes on. S6c is its own commit after S6b, a move only: `rust/xmla/` holds
`xmla/` and `soap/`, its tests (`rust/tests/xmla/` with its fixtures,
`rust/tests/soap/`), `rust/benchmarks/media/xmla.rs`, the `xmla` rows of
`iobase_calls.rs` (`xmla_costs`, `mod service`) and of `allocations.rs`
(`Rowset`); `cli/src/xmla.rs` - `yggdryl xmla serve` - depends on
`yggdryl-xmla`; the Python binding keeps `Xmla` as its handle class, picked
by the codec's name already (`python/src/media/handles.rs`), in the one
native module (D11 option d); Node carries no XMLA door. The crate-private
items the two folders reach by full path are orientation for D6's list,
derived exactly by S6c's scratch `git mv` plus `cargo check`: `iomedia::{own_options,
dimension_options, container_field, container_row_size, read_record_serie}`,
`iobase::{overwrite_arrow_reader_default_with_field,
append_arrow_reader_default, merge_arrow_reader_default, leaf_writer}`,
`serie::from_canonical_rows`, `record_options_fields!`,
`text::{expected_got, elide_to, ERROR_TEXT_LIMIT}`,
`temporal::{parse_timestamp, format_timestamp}`,
`integer::integer_from_text_as`, `xml::{write_element_text,
write_attribute_text, write_fragment, write_leaf_text, write_x_escape,
decode_x_escapes, is_name_start, is_name_char, from_utf, shaped}`,
`warehouse::{holds, no_catalog, path_text}`, `http::server::normalize_path`,
`xxhash::xxh`, `uuid_parse`, `media::DEFAULT_COMMIT_BYTE_SIZE` - about 30.
The name: crates.io answers 404 for `yggdryl-xmla` (read 2026-10-09 through
the crates.io API); release preflight and AGENTS §6's name list gain it in
S6c. Left to S6, one decision for the four leaving media: the rank a
leaving medium's `install()` claims at, since `media::codec::claim` refuses
one below `EXTERNAL_RANK` from outside the core and the order pin states
today's positions. Refused: keeping XMLA a core variant (the user asked
otherwise; the provider, the rowset document and the SOAP envelope are one
protocol with their own options, tests, benchmark and page, and nothing in
the core but the routing vocabulary needs them); folding S2b into S3 (a
media change is the media point's slice, not the seams'); moving `soap/`
alone or leaving it in the core (nothing but XMLA reads it).

### S2b results

Committed as "Register the XMLA medium in place" (on `2d800d51b`): 15 files changed, 123 insertions(+), 129 deletions(-)
across `rust/src` (eight files), `rust/tests` (four), `docs/media/index.md`,
`.api-inventory.txt` and AGENTS.md's three media sentences - the handoff
files apart. The pins held: the three `xmla` hashes of `mod s2_pins`
(`8_486_799_845_904_195_949`, `11_124_752_632_585_582_100`,
`2_087_720_147_871_917_823`) and the order pin, renamed
`the_media_order_by_their_rank` for what it pins (the codec's rank), every
value unchanged; the `xmla` rows of `iobase_calls` (`xmla_costs`, `mod
service`) and of `allocations` (`a_rowset_write_allocates_nothing_per_row`)
unmoved, nothing re-pinned. The review (opus) could not refute the slice
and named three findings, all fixed before the commit: AGENTS.md's three
lines still saying "the core's four media" and listing `XmlaSerie`; no test
reading an XMLA handle through `GenericMediaSerie` or pinning the
`require_settings::<XmlaOptions>` refusal (now
`an_xmla_handle_is_read_through_the_generic_media_serie`); the order pin's
name. It also corrected a sentence: no `read_serie` ever built a media
serie, so "an XMLA handle reads as `GenericMediaSerie`" is "a media serie
over an XMLA handle is `GenericMediaSerie`" here and in the commit.

| Check | Result |
| --- | --- |
| `cargo fmt --all -- --check` | clean |
| `cargo test -p yggdryl --all-targets --all-features --no-fail-fast` | 63 targets, 9170 passed, 1 failed (`market_register`'s count race, S1's: 25 of 200 parallel runs, 0 of 100 single-threaded), 3 ignored |
| `cargo test -p yggdryl --all-targets --no-fail-fast` | 63 targets, 6523 passed, 0 failed |
| `cargo test -p yggdryl-cli --all-targets --no-fail-fast` | 35 passed, 6 ignored |
| `cargo test -p yggdryl --doc` | 628 passed |
| clippy, workspace all features and `-p yggdryl` default, `-D warnings` | exit 0 both |
| `RUSTDOCFLAGS="-D warnings" cargo doc -p yggdryl --no-deps --all-features` | exit 0 |
| Python: maturin develop, pytest (Spark deselected), mypy --strict | 2780 passed, 4 skipped; no issues in 70 files |
| Node: build:debug, the CLI build, npm test, tsc, the generated files | 1120 of 1122 passed, the sandbox `TextDecoder` pair failing as at every slice; tsc 0; loader and declarations unchanged |
| the two docs manifests | current |
| `mkdocs build --strict` | clean |
| the inventories, `generate_internals.py --check` | current |
| the page examples, Rust / Python / JavaScript | 940 passed / 837 run, 3 skipped, 0 failed / 789 run, 2 skipped, 0 failed |

### D34, the medium holds its origin and the serie composes the pushdown (P2)

Decided by the user mid-S2b ("refine so media hold the origin full field
definition and the media serie holding the media is the only wrapper to push
down selectors and filters or other pushable operations optimally"), mapped
by five readers and one synthesis under the scratchpad's `pushdown_map/`
(`field_ownership.md`, `pushdown_sites.md`, `callers_and_pins.md`,
`planned_pushdown.md`, `metadata_caches.md`, `design_inputs.md`). Today the
declared field (`options.field()`, `media/options.rs:1613`) is the caller's
intent and does four jobs - the read's cast target (`transfer.rs:1152`), the
projection vehicle that outranks `apply_columns` (`arrow/mod.rs:718-724`),
the write target (`transfer.rs:1360`) and the root name - and sits inside
the options' identity, so it cannot hold the origin; no medium holds the
origin's whole field (six wrappers cache six different objects, only while
`opened`; text, bare handles, folders and `MediaTable` hold nothing); the one
uniform holder is the serie's `source_field` (`media_serie.rs:52`, filled at
`:111-112`), overwritten by `splice` and `sliced`; `read_arrow_field` under a
`select` answers three different things across its 21 implementations (the
projected field, the whole stored field, the whole field plus Iceberg's
proven `SORT:by`), and Parquet's answer clears the root metadata; a full
declared field defeats `select` in IPC, Parquet, Avro's batch path, Iceberg
and every Hive leaf; the residual (`iomedia.rs:1415`) is copied in five
places (`http/request.rs:1774, 1786`, `iceberg/table.rs:2225`,
`coding/mod.rs:334`, `csv/media.rs:610`); `apply_columns` takes late aliases
and answers `None` on a star; the write-records doors apply `max_row_size`
twice (`iomedia.rs:938-1022` then `transfer.rs:422`).

Decided, one slice (P2, after S2b, before S3 - the P1 precedent for a
user's refinement of the media point, and the codec doors S6 copies are
designed once, D21):

1. The origin door. `IOMedia::read_origin_field(&self) -> Result<Option<Field>>`:
   the whole root the origin holds, its metadata whole, no declaration and
   no clause applied, `None` where the origin states no shape (an empty
   resource, a document carrying no schema); a leaf answers it through its
   codec's `stated_field` under the medium's default options, served from
   the medium's cache (D35), a container through `container_field` as
   today, uncached; every wrapper and `Holder` forward it. Parquet's root
   metadata is kept here, and its `read_field` landing is derived from it.
2. One schema answer. `read_arrow_field(options)` is one rule in the trait
   default - `options.plan().field_from(declared, else the origin)` - the
   declared root as it stands, narrowed by the select, or the origin narrowed
   by it; a `SORT:by` the root declares is kept only where the selection
   keeps every key it names (the hidden risk at `selector.rs:857-860` closed).
   The 21 implementations become the default plus the container arm.
3. One projection rule. What a medium decodes is the declared children (or
   the origin's) intersected with the columns the select and the early
   filter read, in the origin's layout: `apply_columns` answers that set
   through `filter_phases` (a star every column; a late alias nothing), and
   the declared root handed to a codec's `read_batch_reader` is already
   narrowed to it, so the cast target is the narrowed root and a declared
   column outside the selection is never asked for. The media keep reading
   the sections (D21 holds); the sections say less.
4. The composer. `media_serie.rs` holds the one crate-private composition
   from (the medium's options, the serie's `Scan`) to the options the medium
   is handed and the residual - the early/late split made once, a conjunct a
   settled bound proves for every row of a unit dropped from that unit's
   residual (a Hive leaf's path equalities; Iceberg's `file_residual` stays
   its own; a row-group statistic settles nothing), the bounds pushed where
   the medium takes them and kept otherwise, the residual applied once -
   and `read_record_serie` and the serie's `read` both call it, so
   `IOMedia::read_arrow_reader(options)`, the bindings' `where=`/`select=`
   properties and `Plan::execute` are unchanged doors over one composition.
   The five residual copies are redirects or deleted; `source_field` is
   deleted, the serie binding its field through the composed options'
   `read_arrow_field`; `WriteLimitState` is the one owner of the row bounds
   on the write-records doors.
5. S8 amended, not pre-built: `push_plan` is the composer fed from a `Plan`;
   `set_plan` clears nothing (the declared field kept, the two `plan.rs`
   restores deleted); `Serie::from_holder` splits its options through it;
   the lazy root is the medium's cache cell; `delete_from` joins the deleted
   list. The lazy verbs stay S8b's.

Pins: the `s2_pins` byte-identical; every cost row unmoved; the media-serie
construction row (`iobase_calls.rs:1012`) measured red first and allowed to
move only down; new pins: a full declared field with a `select` decodes the
selected columns alone (IPC, Parquet), `read_arrow_field` one answer on
every wrapper under a select, the composer's early/late split and the
residual applied once (a counting filter), the write-records bounds once.
Refused: moving the native pushdown out of the media into the serie (a
Parquet row-group prune reads the footer the medium holds; D21 keeps the
sections the medium reads); a second options type for the serie; deferring
the composer to S8 (the user asked for the one wrapper now; S8b's
`push_plan` is then a thin door over it).

### D35, the medium's cache under a time-to-live (P2)

Decided by the user mid-S2b ("add in media metadata/field cache ttl and
ensure operations and implementations update this cache, 0 is realtime and
other are in millis"). Today six wrappers hand-roll the same cache (`bool
opened`, `OnceLock`s, a private invalidate, their own drop lists:
`ipc/mod.rs:827-833`, `parquet/mod.rs:2252-2262, 2332`,
`avro/batch.rs:2555`, `csv/media.rs:431`, `xmla/media.rs:235-236`,
`excel/media.rs:370`), two drop sites are missing (`set_media_type` on
Parquet and Avro, `handle_mut` on CSV, XMLA and Excel), and a write throws
away what it knows: Parquet's writer returns the footer it wrote and the
wrapper drops it and reads it back (`parquet/mod.rs:717, 2437`), the
overwrite returns the published field only IPC consumes
(`transfer.rs:257-312`).

Decided:

1. `media/cache.rs` `MediaCache`, one cell per medium wrapper (IPC, Parquet,
   Avro, CSV, text, XMLA, Excel): `Mutex<Option<Entry>>` with `at: Instant`,
   `generation`, `origin: Option<Field>`, `rows: Option<u64>`, `columns:
   Option<usize>` and `state: Option<Box<dyn Any + Send + Sync>>`, the
   medium's own object (Parquet's `ParquetMetaData`, Avro's dimensions,
   CSV's inferred options and field, the Excel `Workbook`), reached by
   downcast as `IOMedia::as_any` answers the Parquet footer (S2). It replaces
   the six caches and closes their drop-site gaps; `ParquetFooter` reads it.
2. The rule, one sentence: an entry is served while the handle is open, or
   while it is younger than the TTL; `cache_ttl` `0`, the default, is
   realtime - a closed handle re-reads on every ask, as today - and `n`
   serves an entry younger than `n` milliseconds. `open` holds the entry
   until `close`, as the AGENTS rule says ("open caches expensive metadata
   for its scope, close publishes and drops it"), the TTL being the time
   rule outside an explicit session. The clock is `Instant::now()` read at
   the door and passed in (`Buffered::read_at`'s model), injectable under
   `internals`; the open-scope pins (about 35 tests, three docs tables) hold.
3. The setting: `cache_ttl`, `u64` milliseconds, a shared section of every
   medium's options through `record_options_fields!`, outside the options'
   identity and the hash feed as `file_threads` is (`FileThreads` model:
   equality always true, `Hash` writes nothing; `ParquetOptionsIdentity`
   omits it) so the `s2_pins` hold byte for byte; read from text through the
   one integer grammar (`integer_from_text_as`, as Iceberg's
   `commit.retry.*-ms`), never `duration_from_text`; `IORecordOptions::
   cache_ttl`/`set_cache_ttl`, the `MediumOptions` twin, the Python property
   `cache_ttl` (an `int` of milliseconds; Python's `buffered(ttl=)` is
   seconds, said once in the docs), the `**properties` per-call copy, the
   pickle; Node gains no door (AGENTS §4).
4. Writes update the cell with what they know: an overwrite sets the origin
   to the published root (`transfer.rs:257`'s returned field), `rows` to
   `written_rows` and `state` to what the encoder returned (Parquet's
   `overwrite_buffered` metadata kept, its tail re-read gone); an append
   keeps the origin, adds `written_rows` to a fresh `rows` and clears a stale
   one, clears `state` the encoder did not answer; `clear` sets the empty
   entry (no origin, zero rows); `remove`, `merge` (which counts every
   pulled row), `pwrite`, `truncate`, `create_bytes`, `set_media_type` and
   `handle_mut` invalidate it (the generation bumped); a closed write stores
   an entry only under a TTL above zero. Containers stay uncached. Iceberg's
   document and manifest list are the table format's own cache with its
   commit `adopt`, an update on write already; whether the TTL governs them
   is recorded open for S6.
5. Pins: new rows under an injected clock - a warm closed read is free under
   a TTL, a write answers its field with zero reads on every wrapper, an
   out-of-band change is seen after the TTL and not before, each wrapper's
   drop sites (the two missing ones included); the Parquet overwrite row
   moves down by its footer re-read, red first; a Python round trip of the
   property. Refused: the TTL on the handle as a `Buffered`-style decorator
   (it sees the three answers and not the footer the decode reuses, and
   adds a settings family beside the options against D23); the TTL as the
   open scope (it reverses D19, D21 and D26 and moves 35 pins for nothing
   the rule above lacks).

### P2 results

Committed as "Hold the origin field and its cache on the medium; compose
the pushdown once" (on `d100439d3`). Five workers wrote the slice in the
`wip/p2` worktree with no compiler (A the core, B the seven wrappers, C the
readers that are not wrappers, D the tests, E the bindings, the docs and
the inventories); it was merged onto the program branch, conflicting only
in AGENTS.md (the `media_serie.rs` row, the `ipc/` row beside the new
`media/cache.rs` row) and `docs/media/index.md` (the Read paragraph), both
resolved to the accepted design below; every Rust file merged clean.

| figure | value |
| --- | --- |
| the slice, the handoff files apart | 70 files, +7755 / -1377: `rust/src` 34 files +3090/-1260, `rust/tests` 18 files +4285/-42, Python 5 files +182/-6, docs, skills, AGENTS.md and the inventories 13 files +198/-69; Node and the CLI untouched |
| new files | `rust/src/media/cache.rs` 323 lines (`CacheTtl`, `Entry`, `MediaCache`, the clock), `rust/tests/media/cache.rs` 466 |
| tests | all-features lane 9298 passed (S2b 9170), default lane 6580 (6523), Python 2784 passed (2780) |
| caches | six hand-rolled ones (IPC, Parquet, Avro, CSV, XMLA, Excel) replaced and the text wrapper given one, one `MediaCache` per wrapper; the two missing drop sites (`set_media_type` on Parquet and Avro, `handle_mut` on CSV, XMLA and Excel) pinned by each wrapper's `every_byte_verb_drops_...` tests |
| residual copies | five gone (`http/request.rs` two, `iceberg/table.rs`, `coding/mod.rs`, `csv/media.rs`) and `RecordOptions::apply_stream`: `compose` and `Residual` in `media_serie.rs` are the one owner |

The design departures, accepted by the coordinator before the settle (the
first four were worker A's) and kept by the review:

1. The default `read_arrow_field` reads the leaf's stated field through the
   codec of the options it is given - the declared root, else the stated
   one, then `field_under(options, root)` - not through the medium's own
   `record_options`, which on a bare handle is two questions more; a
   wrapper answers the same rule over its cached origin
   (`held_arrow_field`, the one body of IPC, Parquet and Avro).
2. The serie keeps `root`: D34.4 deleted `source_field` and bound the
   serie's field through the composed options; built, the serie keeps the
   root it bound - the medium's declared field, else its origin - so a
   re-plan reads nothing, its field `field_under(options, root)`.
3. The residual holds the whole `where` and splits it before and after the
   selection against the reader's own schema where it is applied, rather
   than carrying the half `compose` split: a medium prunes by its early
   half and filters no row, so the rows the residual meets are unfiltered.
4. The medium's read door composes once: the serie hands the medium its own
   options with only what its verbs stated laid over them, and the door
   (`read_record_serie`) composes, so no second composition sits in the
   serie.
5. `RecordOptions::apply_stream` is deleted: `Residual::apply_stream` is the
   one native-row shaping, reached by its pull pin through
   `yggdryl::internals::media_serie::apply_stream`.
6. `open()` drops an entry a closed handle held under a TTL, so a session
   serves what the store states from its start; opening an open handle keeps
   its entry. D35.2 said only that `open` holds the entry until `close`.
7. A folder is read per unit: each leaf composes with the conjuncts its path
   settles (`of_unit`) and the folder's residual runs once over the units
   (`over_units`), Iceberg's `file_residual` early and `over_units` the rest.

The settle, on the merged tree (`cargo check` clean in every lane after
one fix; the smoke rows in the all-features lane):

1. `rust/tests/media_register.rs`' `test_options!` lacked `cache_ttl`: the
   field added; `held_arrow_field` made the one body of three wrappers.
2. `root/iomedia.rs` `native_rows_stop_at_the_global_row_limit_without_one_extra_pull`
   measured 4 pulls against 3: the write-records row reader had lost its
   cap, a pull bound moving up - fixed at its cause (`records_pull_bound`
   hands the reader `row_offset + max_row_size` where every pulled row is
   one the bound counts, `WriteLimitState` still the one owner of the exact
   trim), never re-pinned.
3. `ipc/mod_.rs` `a_late_alias_reads_no_stored_column_and_a_star_reads_them_all`
   (new) read with no declared field, and an IPC projection rides a declared
   root: re-spelled under the full declared field, the D34.3 rule it pins.
4. `xmla/options.rs` `a_section_the_plan_does_not_spell_is_cleared`, moved by
   D34.5: re-spelled `..._and_the_declared_field_stands`, asserting the
   declared field kept.
5. `media_serie.rs` `settles`: a `year = 'null'` conjunct against a
   `year=null` directory was proven through `partition_text` and dropped; a
   null on either side now proves nothing - `media/partition.rs`
   `a_path_settles_the_equalities_it_proves_and_a_null_directory_proves_none`,
   red without the guard.
6. New pins for worker C's `with (cache_ttl = ...)` knob
   (`expression/plan.rs` `a_target_reads_its_cache_ttl_as_whole_milliseconds`:
   `1s`, `1.5`, `-1`, `soon` refused at `$.with.cache_ttl`) and the text
   wrapper's cache (`text/handle.rs` `mod ttl`, three tests).

The review (the `code-review` skill at high effort over the whole diff,
plus the manager's pass over the brief's checklist) named fourteen points:

| # | point | outcome |
| --- | --- | --- |
| 1 | `compose` narrowed a declared root a headerless medium (CSV, Excel) pairs by position, so `select c` over `(a, b, c)` read column a | fixed: such a root is handed whole; `csv/media.rs` `without_a_header_a_selection_reads_each_declared_column_from_its_own_position`, red without the guard |
| 2 | `clear()` over a container stored the empty entry, served after a later folder write | fixed: `clear` keeps the empty entry for a leaf alone |
| 3 | IPC, Parquet and Avro `read_arrow_field` accepted another encoding's options | fixed: `require_record_options` restored before `held_arrow_field` |
| 4 | `Parquet::size()` served the cached length on a closed handle under a TTL, and the default `append_bytes` offsets by `size()` | fixed: a byte length is served while open alone |
| 5 | CSV and Excel `row_size` never kept a count learned on a closed handle under a TTL | fixed: `MediaCache::add(ttl, now, edit)` lays it on the served entry, its stamp kept |
| 6 | CSV `row_size` on a miss reads the document for the count and the sample for the origin | kept: the second read is the bounded inference sample |
| 7 | `update` creating a default entry stood for "no shape" | fixed with 5: read-learned facts go through `add`, `update` stays the write doors' |
| 8 | seven copies of the `keeps(ttl)` rule | fixed: one `MediaCache::keeps(ttl)` |
| 9 | `ParquetFooter::held` swallows a fill error, so a root this crate cannot type reads the file's end twice per read under a session or a TTL | kept: an error path, the decode still reports the failure |
| 10 | the Python `plan` setter docstrings said every section is replaced | fixed: every one but the declared field, which a plan with no `create` section leaves standing |
| 11 | `container_origin` copied five times and inlined twice | fixed: one `iomedia::container_origin(handle, options)` |
| 12 | Python `cache_ttl` refused a NumPy integer | fixed: any `__index__` integer; the stub `SupportsIndex \| str \| None` |
| 13 | Python `cache_ttl=None` was a `TypeError` | fixed: `None` is a value and clears, to `0` |
| 14 | `Text::opened()` answers the wrapper's session, not the handle's | kept: as the six other wrappers did at HEAD |

Known edge, recorded: under a declared root whose `PARTITION:by` names a
column the `select` drops, the narrowed root handed to the codec prunes that
entry while `read_arrow_field` keeps the root's metadata.

Pins. The `s2_pins` are byte-identical (`CacheTtl` hashes nothing,
`ParquetOptionsIdentity` omits it) and no cost pin moved: `iobase_calls` 67
passed, its one new row `a_closed_wrapper_under_a_ttl_answers_a_warm_ask_with_no_call`,
the media-serie construction row measured unmoved (`pstream_bytes=1
media_type=2 is_container=2`), its doc now saying why - the serie keeps the
root it bound; `allocations` 184 passed, unmoved. The Parquet overwrite's
footer re-read is gone, pinned by `an_overwrite_answers_the_origin_and_the_rows_with_zero_reads`
and `a_read_after_a_ttl_overwrite_reads_no_footer_and_a_realtime_one_reads_it`;
no `iobase_calls` row measured a write, so none moved. Re-spelled with
their reason: the XMLA `set_plan` test above (D34.5); the pull pin of the
deleted `RecordOptions::apply_stream`, moved into `root/media_serie.rs`'s
`internals` module over `Residual::apply_stream`, its three pulls
unchanged; the `RowMedia` double's own `read_arrow_field` deleted for the
default; the Avro and Parquet "a closed write must not start a cache"
messages now say "session"; the Python identity test calls `stable_hash()`
rather than comparing the bound methods.

| check | result |
| --- | --- |
| `cargo fmt --all -- --check` | clean |
| `cargo check -p yggdryl --all-targets`, with `--all-features`; `cargo check --workspace --all-targets --all-features` | clean, 0 diagnostics |
| the smoke, all-features lane, after the review's fixes (`--test <t>`) | media 205, root 1871, ipc 52, parquet 88, avro 142, csv 102, xmla 761, excel 332, text 236, iobase_calls 67, iceberg 528, expression 244, http 503, warehouse 139, allocations 184 passed; media_register 14; 0 failed |
| `cargo test -p yggdryl --all-targets --all-features --no-fail-fast` | 63 targets, 9298 passed, 0 failed, 3 ignored |
| `cargo test -p yggdryl --all-targets --no-fail-fast` | 63 targets, 6580 passed, 0 failed |
| `cargo test -p yggdryl-cli --all-targets --no-fail-fast` | 35 passed, 6 ignored |
| `cargo test -p yggdryl --doc` | 628 passed |
| clippy, workspace all features and `-p yggdryl` default, `-D warnings` | exit 0 both |
| `RUSTDOCFLAGS="-D warnings" cargo doc -p yggdryl --no-deps --all-features` | exit 0 |
| Python: maturin develop, pytest (Spark deselected), mypy --strict | 2784 passed, 4 skipped; no issues in 70 files |
| Node: build:debug, the CLI build, npm test, tsc, the generated files, `test:package:debug` | 1120 of 1122 passed, the sandbox `TextDecoder` pair failing as at every slice; tsc 0; loader and declarations unchanged; the package audit passed |
| the two docs manifests | current |
| `mkdocs build --strict` | clean |
| the inventories, `generate_internals.py --check` | current (180 source files and 587 `pub` names not described) |
| the page examples, Rust / Python / JavaScript | 940 passed / 837 run, 3 skipped, 0 failed / 789 run, 2 skipped, 0 failed |
| no test code under `src/` | the grep is empty |
| local-only checks (charset tables and interop, ISIN seed, country and MIC tables), benches | not run: nothing the slice touches makes them stale |

## S2: what was built

The media extension point, in place: four claim-once registers on
`plugin::Register`, each seeded by the core with its own implementations,
the three leaving media (Parquet, Avro, the workbook), the Iceberg format,
the `hadoop` and `s3tables` catalogs and the `s3tables` locator claimed where
they stand, and every dispatch seam in the core reading the claim rather than
naming a medium. What the slice decided beyond the design is under "The
review's amendments (S2)" above; the design's D26-D32 hold otherwise.

### S2 results

Counted on the slice's tree (`git grep`, `wc`), checks as the chain logs
under the scratchpad's `logs/chain_s2*.log` report them.

| figure | value |
| --- | --- |
| retired names in `rust/`, the bindings and the CLI (`Media::Parquet`, `RecordOptions::Avro`, `Table::Iceberg`, `Error::Iceberg`, `parquet_footer`, `iceberg::located`, the four series, the eleven accessors, `__delegate_iomedia_parquet`) | 0 (one test *named* after the footer stays) |
| `RecordOptions` variants | 5: `Ipc`, `Text`, `Xmla`, `Csv`, `Registered` (7 at `719299cf6`) |
| `Media` variants | 5: the same four and `Registered` (7) |
| `Serie` variants | 78 (82: `Parquet`, `Avro`, `Excel`, `IcebergTable` gone) |
| core-variant matches on a medium outside its own file (`RecordOptions::Text`, `::Csv`, `::Ipc` in `rust/src`) | 17, all the Text and CSV core rules the design keeps (the text leaf, the CSV header target, their native appends, the container scan's text-last rule, the ISIN store's text layout, the HTTP and coding defaults) |
| codec statics / format, factory and locator statics | 7 / 4 |
| `cfg(feature = "iceberg")` and `cfg(feature = "parquet")` in `rust/src` outside `iceberg/`, `s3tables/`, `parquet/` | 43 (191 at `719299cf6`) |
| `cfg(feature = "s3tables")` in `rust/src` outside `s3tables/` | 10 (27) |
| new files | `media/codec.rs` 310 lines, `media/format.rs` 261, `holder/locator.rs` 109, `rust/tests/media_register.rs` 821 (14 tests over five test-only implementations) |
| the slice | 107 files, +5178 / -2449: `rust/src` 51 files +3290/-2003, `rust/tests` and benchmarks 24 files +892/-195, Python, Node and the CLI 10 files +157/-109, docs, skills, `AGENTS.md`, `README.md` and the inventories 21 files +564/-141 |
| binding sites through the typed settings door / through `downcast_ref` | 31 / 22 |
| the S2 pins (`rust/tests/media/options.rs` `mod s2_pins`): every medium's default `stable_hash`, the shared-section feed, the own-setting feed, the variant order | unmoved, green in both lanes |
| cost pins (`iobase_calls`, `allocations`, the Iceberg `call_counts`, the S3 Tables request counts) | unmoved, green in both lanes; no number re-pinned |

| check | result |
| --- | --- |
| `cargo test -p yggdryl --all-targets --all-features --no-fail-fast` | 63 targets, 9170 passed, 0 failed |
| `cargo test -p yggdryl --all-targets --no-fail-fast` (default features) | 63 targets, 6522 passed, 0 failed |
| `cargo test -p yggdryl-cli --all-targets --no-fail-fast` | 35 passed |
| `cargo test -p yggdryl --doc` | 628 passed |
| `cargo clippy --workspace --all-targets --all-features --no-deps -- -D warnings`; `cargo clippy -p yggdryl --all-targets --no-deps -- -D warnings` | clean in both lanes (after one `matches!` rewrite in `error.rs`) |
| `RUSTDOCFLAGS=-D warnings cargo doc -p yggdryl --no-deps --all-features` | clean (after six intra-doc link lints: three links to crate-private items spelled as code, three redundant explicit targets) |
| `cargo fmt --all -- --check` | clean |
| `maturin develop`, `pytest python/tests` (Spark interop deselected), `mypy --strict` | 2780 passed, 4 skipped (pyspark, two PEP 649 tests, one free-threaded test); mypy no issues in 70 files |
| `npm run build:debug`, `cargo build -p yggdryl-cli`, `npm test`, `tsc --noEmit`, generated loader and declarations | 1120 of 1122 pass; the two failures are `node/tests/charset.test.js` decoding windows-1252's C1 range through this sandbox's Node 22.22 `TextDecoder`, which CI's Node passes (as in S1); tsc clean; `node/index.js` and `node/index.d.ts` unchanged |
| `node scripts/build_docs_fix.js --check`, `build_docs_playground.js --check` | current |
| `mkdocs build --strict` | clean |
| `python scripts/check_api_inventory.py`; `generate_internals.py --check` | current (180 source files and 586 `pub` names not described, as before) |
| `check_docs_examples.py --lang rust` / `python` / `javascript` | 940 passed; 837 run, 3 skipped, 0 failed; 789 run, 2 skipped, 0 failed |
| local-only checks (charset tables, charset interop, ISIN seed, country and MIC tables) | not run: nothing the slice touches makes them stale |
| disk | the whole run of one lane holds about 11 GB of test binaries under the session's allowance; the chain cleans the workspace's own artifacts between lanes and before the bindings, after a first chain died on a full disk |

## S3: design

The remaining seams in place, decided against the four maps under the
scratchpad's `s3_map/` (`market_fix_reach.md`, `media_reach.md`,
`d5_d9_d10_sites.md`, `pins_docs.md`) and their synthesis
(`design_inputs.md`), read at `2d800d51b`. The core reaches 115
crate-private items for the market and FIX crates (91 by path, 21 by method
call, 3 only inside macro expansions); the D9 cycle is one type, `FixMsg`,
named by `graph/market_data.rs` alone; of the 116 core items the media
folders reach, S3 forwards the 17 that market or FIX code also reaches and
S6 cuts the other 99. S3 needs no answer from the user.

- **D5, refined.** Under D25 the eighteen code blocks stay; the four
  `impl DataType` blocks in scope are `side.rs:293`, `timeinforce.rs:141`,
  `marketdatakind.rs:279`, `marketdatatype.rs:547`. The spelling is an
  inherent `pub const fn dtype() -> DataType` and a nullable `pub fn
  field(name) -> Field` in `enum_leaf!`'s market arm (`enums.rs:307-315`),
  one edit for the four kinds, legal in the invoking crate after S4, and
  the inherent item is what keeps `Side::dtype()` unambiguous where `Value`
  and `MarketValue` are both in scope. The four blocks go; `DataType::side()`
  and its three siblings are deleted and every site re-spelled (252 lines:
  tests 132, docs 80, skills 7, src 17, python 6, node 4, benches 3, cli 1,
  inventory 5, AGENTS 1); `MarketValue::dtype()` has no caller and is
  deleted. The `DataType` values are identical, so the crate dump and the
  dictionary hash hold.
- **D9, refined.** The trait is `MarketMessage`, in `graph/market_data.rs`:
  supertraits `Event + Operation + Debug + Send + Sync + 'static`, the
  by-value forms of the walk and merge arms as `self: Box<Self>`
  (`into_market_data`, `into_market_leaf`, the following and merging arms),
  `stable_hash`, and the object-safe twins `clone_box`, `dyn_eq`, `as_any`,
  `into_any` (the `MediumOptions` precedent); `MarketData::Fix` holds
  `Box<dyn MarketMessage>` and `MarketKind::Fix` stays (D10's vocabulary);
  `MarketData::as_fix` becomes `downcast_ref::<T>()`, the bindings keeping
  `as_fix` through it; the `From`/`TryFrom` impls move into `fix/`.
  `MarketData` stays 912 bytes and the `allocations` rows at `:922`,
  `:937`, `:1695` hold. The protocol-view builder is the existing
  `protocol_field_types!` exported `#[macro_export] #[doc(hidden)]` with
  `($vis, $scheme, $View, $ViewMut, $label)`, its one private field read
  replaced by a public `ProtocolFieldMut::as_field`; the core's 23 views
  pass `pub(crate)`, `fix/field.rs` passes `pub` and holds `FixField`/
  `FixFieldMut`; the FIX entry of `for_each_well_known_protocol!`
  (`metadata.rs:156-163`) goes and with it `Field::as_fix`/`as_fix_mut` and
  `Metadata::as_fix`; every caller spells `FixField::new(&field)` (a
  1,684-line script). Pins: the dictionary hash
  `14_542_711_836_201_211_247`, the census, the five-document crate dump,
  `market_register.rs`, `root/market.rs`, the S0 hashes, all unmoved.
- **D10, refined.** Stays core, with the reason: `Scheme::FIX`
  (`metadata.rs:409-431` reads a known scheme's property allocation-free;
  a custom scheme would allocate per read and break `allocations.rs:789`);
  the `MimeType` FIX words; the code names and listings (D25); `STATE_CODES`
  (`State::from_spelling` reads it) with one forwarder; the FIX temporal
  readers, `pub(crate)` behind forwarders (one flag-selected `clock_at` body
  over private helpers, not an API); `parallel.rs`, reached through D6.
  Moves into `fix/`: State's FIX doors as free functions in `fix/state.rs` -
  `from_status`, `from_msgtype`, `STATUS_TAGS` and the seven tables
  (`state.rs:319-504`) - their callers `fix/msg.rs:180, :2388-2394`,
  `native_derivations.rs:297` and the Python natives (`python/src/state.rs:
  47-56`, Python's surface unchanged until S5; Node has none) redirected,
  the tests to `rust/tests/fix/state.rs`; the FIX view (D9); the 32
  FIX-Latest logical names as a fix-owned table the core seed claims until
  S4 (the listing pin holds, `logical_names()` sorts). Logging: a logger is
  named by its module path under `yggdryl` whatever crate holds the module,
  through a per-crate table at `logging/facade.rs` `is_foreign`/`logger_for`
  (`yggdryl_market` -> `yggdryl`, `yggdryl_<folder>` -> `yggdryl.<folder>`,
  so `yggdryl_fix::build` logs as `yggdryl.fix.build`), `yggdryl_cli` foreign
  as `tests/logging/facade.rs:84-90` pins, the table pinned with synthetic
  targets. Nothing of the market crate moves in S3.
- **D6, refined.** One root file, `implementer.rs`, `pub mod` at the crate
  root, in sections - both crates, market, fix, the 17 media-shared - each
  item by its route: R, raised to `pub` inside a crate-private module and
  `pub use`d (38); F, an `#[inline]` forwarder for an item of a published
  module (40); A, a free forwarder over a `pub(crate)` inherent item (25);
  M, the definition moved into `implementer.rs` (`InstantSequence`,
  `Staged`, 6); X, `#[macro_export] #[doc(hidden)]` with expansions spelled
  `$crate::implementer::..` (`warned!` with `warning::warn`, `enum_leaf!`
  with `enums::Patterns`, 6). Eleven `pub(super)` functions of
  `graph/element.rs` are raised to `pub(crate)` first;
  `Field::new_with_metadata` (`field.rs:976`) joins the list. Every new pub
  item carries its doc line (`missing_docs` under `-D warnings`);
  `Proof::Proven` never crosses. The list is proven without S4's crates:
  every leaving-file site re-pointed to `crate::implementer::X` by one
  exact-string script, `cargo check --workspace --all-targets
  --all-features --keep-going` clean, and `scratchpad/work/refs.py` re-run
  answering zero non-pub cross-crate paths; S4's `cargo check -p` finds the
  method-reached residue. The 31 market-owned items FIX reaches (15 paths,
  16 methods) cannot be published inside `pub mod graph` and wait for
  `yggdryl_market::implementer` in S4, which is therefore not moves-only for
  them. Refused: a raise plus `#[doc(hidden)] pub use` inside a published
  module (the AGENTS rule); a per-file `implementer` module gathered by a
  generator (about forty files and a generator for thirteen raises saved).
- **Order inside the one commit:** D5 (the macro arm, then the rename),
  D10, the D9 trait, the D9 view (the builder, then the `as_fix` script),
  D6 (forwarders, raises, moves, the re-point script, the static proof),
  the bindings, the docs; each phase settled by `cargo check --workspace
  --all-targets --keep-going --message-format=short` and its suite, the
  whole run leading the chain. The stale sentences of D4, D5, D6 and D10
  above are corrected by this slice's docs phase against these counts
  (`design_inputs.md` B5).

## S3: what was built

The remaining seams in place (D5, D6, D9, D10), as "S3: design" decided, with
nothing of the market or FIX code moved. Stage 1 wrote the definitions in the
worktree `wip/s3` on six disjoint file sets with no compiler; stage 2 swept
the call sites with three exact-string scripts (`scratchpad/s3_sweep/d5.py`,
`d9.py` over `d9_plan.py` and `overrides.py`, `d6.py`); the lane manager
pre-settled the worktree under its own check target while P2 landed, then
squash-merged it onto P2 (one conflict: the `AGENTS.md` Layout rows).

- **D5.** `enum_leaf!`'s market arm writes `pub const fn dtype() ->
  DataType` and the nullable `pub fn field(name) -> Field` on the four kinds;
  `DataType::side()`, `timeinforce()`, `marketdatakind()`, `marketdatatype()`
  and `MarketValue::dtype` are deleted. The sweep re-spelled 158
  occurrences (docs 77, tests 60, Python 6, skills 6, src 5, benchmarks 3,
  CLI tests 1; nine nullable `Field::new(.., DataType::side(), true)` became
  `Side::field(..)`, `allocations.rs` keeping its form as a cost pin) and the
  test worker 72 in its own files.
- **D10.** `fix/state.rs` holds `from_status`, `from_msgtype`, `STATUS_TAGS`
  and the seven tables, byte-identical; `State::from_spelling` reads
  `STATE_CODES` through `state_from_wire_code`, its one reader; Python's
  `State.from_fix_status`/`from_fix_msgtype` redirect unchanged until S5. The
  32 FIX Latest logical names are `fix::LOGICAL_NAMES`, which the core's seed
  claims beside its own 19 (`logical_names()` byte-identical). The logging
  facade names a workspace crate's module under `yggdryl` through one table
  (`CRATES`: `yggdryl` and `yggdryl_market` -> `yggdryl`, `yggdryl_avro`,
  `_excel`, `_fix`, `_iceberg`, `_parquet`, `_xmla` -> `yggdryl.<folder>`, the
  bindings and `yggdryl_cli` foreign), and a `warned!` key is that logger name
  (`facade::write_logger_name`, built from no text), so a module keeps its
  logger and its warning counts whatever crate holds it.
- **D9.** `MarketMessage` (supertraits `Event + Operation`, the sealed
  `EventOperation` seam, `Debug + Send + Sync + 'static`; the boxed
  `into_market_data`, `into_market_leaf`, `with_previous`, `merge_with`,
  `restating`, and `stable_hash`, `clone_box`, `dyn_eq`, `as_any`,
  `into_any`); `MarketData::Fix(Box<dyn MarketMessage>)`,
  `MarketData::as_message::<T>()`; `impl MarketMessage for FixMsg` and the
  `From`/`TryFrom` pair in `fix/market.rs`; `MarketData` stays 912 bytes and
  has no `Hash`, as at HEAD. `protocol_field_types!` is exported
  (`#[macro_export] #[doc(hidden)]`, `($vis, $scheme, $View, $ViewMut,
  $label)`, its expansion over public doors and plain code spans); the core's
  23 views pass `pub(crate)`, `fix/field.rs` mints `FixField`/`FixFieldMut`
  `pub`; `Field::as_fix`/`as_fix_mut` and `Metadata::as_fix` are gone. The
  sweep re-spelled 1,664 sites (`rust/tests/fix` 959, `rust/src/fix` 325,
  docs 113, benchmarks 57, Python 51, Node 44, CLI tests 31, `rust/tests` 31,
  skills 29, CLI 18, CLI benchmarks 5, examples 1) and the test worker 19;
  the bindings keep `MarketData.as_fix`/`asFix` through `as_message`.
- **D6.** `implementer.rs`, `#[doc(hidden)] pub mod` at the crate root, 114
  names in four sections (both crates, market, FIX, shared with the media
  crates), by route: R 33 (`pub use` of items raised inside unpublished
  modules, `Patterns` and `warning::warn` among them), F 47 (37 `#[inline]`
  function forwarders and 10 constants), A 25 (free forwarders over
  crate-private associated items, `<type>_<item>`), M 2 (`InstantSequence`
  and `Staged` moved in with their methods), X 7 (`warned!`, `enum_leaf!`,
  `define_field_types!`, `protocol_field_types!`, `bytes_dtypes!`,
  `bytes_scalars!`, `string_scalars!`, exported `#[doc(hidden)]`, their
  expansions through `$crate::implementer`). Against the design's 115: `Proof`
  is not exported (`land_unproven_batch` fixes `Proof::Unproven`), four R
  routes became forwarders where a private inline module stood between
  (`str_from_value`, `civil_from_days`, `percent_decode`,
  `write_named_bytes`), and `MarketDescriptor::adopt_code`,
  `Patterns::read` and `state_from_wire_code`, reached only by expansion or
  method call, joined. The sweep re-pointed 339 sites (`rust/src/fix` 250,
  the market root files 51, `graph/` 29, `isin_registry/` 9). The static
  proof: `scratchpad/work/refs.py` (copy `s3_refs_merged/`) on the merged
  tree answers 0 market or FIX references to a crate-private core path, and
  26 FIX references over 15 crate-private market names - S4's
  `yggdryl_market::implementer`.

### S3 results

| figure | value |
| --- | --- |
| the slice | 197 files, +7220 / -4202 before the review's fixes: `rust/src` 82 files +2866/-1697 (new: `implementer.rs` 963 lines, `fix/state.rs`), `rust/tests` 63 files +3393/-1667 (new: `root/implementer.rs`, `fix/state.rs`), benchmarks 7, Python 7, Node 4, CLI 7, docs 17, skills 5, `AGENTS.md`, `.api-inventory.txt` +157/-28 |
| retired spellings left (`DataType::side()` and siblings, `as_fix(`/`as_fix_mut(` on a field, `MarketValue::dtype`, `State::from_fix_status`/`from_fix_msgtype`/`FIX_STATUS_TAGS`) | 0 outside `.handoff/`, the bindings' own names (`State.from_fix_status`, `MarketData.as_fix`/`asFix`) and prose saying there is no `as_fix` |
| pins | the dictionary hash `14_542_711_836_201_211_247`, the census, the crate dump, the S0 hashes, `MarketData` at 912 bytes, `logical_names()`, every `allocations` and `iobase_calls` row: green, no number edited |
| the review (`code-review` at high) | five findings: the `implementer` module hidden (`#[doc(hidden)]`; the forwarders that skip a shape check are the design's door and hide no `unsafe`), `define_field_types!`'s market arm no longer links core items from an invoking crate, the `warned!` key mapped as D10 states (pinned in `rust/tests/logging/warning.rs`); kept: the explicit `CRATES` table (decision (a): a binding is foreign, so no blanket `yggdryl_*` rule - P3/S6d adds `yggdryl_s3`), and `enum_leaf!`'s absolute `::smol_str` (the market crate depends on `smol_str`) |

### S3 checks

Every command ran on the merged tree from `/home/user/yggdryl` with
`CARGO_INCREMENTAL=0` and the debug info off: the phase suites, then
`logs/chain_s3.sh` under the scratchpad, then `logs/chain_s3b.sh` over the
review's last edits (two doc links, the unused `pub(crate) use warned`, two
test assertions).

| check | result |
| --- | --- |
| `cargo check --workspace --all-targets --all-features --keep-going --message-format=short` (the worktree before the merge, then the merged tree) | 2 errors and 3 warnings in the worktree (two `FixField::new(&document)` borrows, three unused imports), then clean |
| phase suites, all features: `--test root` filtered to the kinds, `state`, `vocabulary`, `datatype`, `implementer`, `protocol`, `market`; `--test graph`; `--test fix`; `--test market_register`; `--test logging`; `--test allocations` and `--test iobase_calls` filtered | 424 of 426 (two of the test worker's assumptions, corrected: 34 of 34 `implementer`), 439, 1026 (the dictionary hash and the crate dump among them, unedited), 7, 88 then 89, 16, 5 |
| `refs.py` over the merged tree | 0 market or FIX references to a crate-private core path; 26 FIX references over 15 crate-private market names |
| `cargo test -p yggdryl --all-targets --all-features --no-fail-fast` | 63 targets, 9357 passed, 0 failed, 3 ignored |
| `cargo test -p yggdryl --all-targets --no-fail-fast` (default features) | 63 targets, 6637 passed, 1 failed - the `implementer` test asserting Iceberg's refusal, which no claimed format makes without the feature; the assertion dropped and the target re-run in `chain_s3b` |
| `cargo test -p yggdryl-cli --all-targets --no-fail-fast` | 6 targets, 35 passed, 0 failed, 6 ignored |
| `cargo test -p yggdryl --doc` | 628 passed |
| `cargo clippy --workspace --all-targets --all-features --no-deps -- -D warnings`; `cargo clippy -p yggdryl --all-targets --no-deps -- -D warnings` | the first clean; the second refused the then-unused `pub(crate) use warned` (deleted) |
| `RUSTDOCFLAGS="-D warnings" cargo doc -p yggdryl --no-deps --all-features` | four redundant explicit link targets the sweep's imports made (`fix/mod.rs`, `graph/market_column.rs`), fixed |
| `cargo fmt --all -- --check` | clean |
| `maturin develop`, `pytest python/tests --deselect python/tests/test_spark_interop.py`, `mypy --strict` | installed; 2784 passed, 4 skipped; no issues in 70 files |
| `npm run --prefix node test:package:debug`, `cargo build -p yggdryl-cli`, `npm test --prefix node`, `tsc --noEmit`, `git diff -- node/index.js node/index.d.ts` | the package audit passed; 1122 tests, 1120 passed, the two sandbox `TextDecoder` tests failing as at every slice; tsc clean; the generated files unchanged |
| `node scripts/build_docs_fix.js --check`, `build_docs_playground.js --check` | current |
| `mkdocs build --strict` | clean |
| `check_api_inventory.py`; `generate_internals.py --check` | current (180 source files and 570 `pub` names not described yet); `internals` current |
| `check_docs_examples.py --lang python` / `javascript` / `rust` | 837 run, 3 skipped, 0 failed; 789 run, 2 skipped, 0 failed; 940 passed |
| `grep -rn '#\[cfg(test)\]\|#\[test\]\|mod tests' rust/src python/src node/src cli/src` | empty |
| `chain_s3b`: fmt, clippy in both lanes, `cargo doc` at `-D warnings`, `--test root --test logging` in both lanes and `--test s3` | fmt clean; clippy clean in both lanes; `cargo doc` clean; default features `--test root` 1727 and `--test logging` 82 passed; all features `--test root` 1911, `--test logging` 89, `--test s3` 264 passed; 0 failed |
| the review: the `code-review` skill at high effort over the whole diff | five findings: three fixed, two kept with their reason ("S3 results" above) |
| local-only checks (charset tables and interop, ISIN seed, country and MIC tables), benches | not run: nothing the slice touches makes them stale |

### For S4 and S6d

- `yggdryl_market::implementer` publishes the 15 crate-private market names
  the FIX code reaches (26 references, `s3_refs_merged/edges.json`), and
  S4's `cargo check -p yggdryl-fix` finds the method-reached residue.
- `enum_leaf!` expands to `::smol_str` absolutely: the market crate keeps
  `smol_str` as a direct dependency.
- The logging facade's `CRATES` lists every workspace crate by name, a
  binding being foreign: the slice that creates a crate adds its row -
  `yggdryl_s3` -> `yggdryl.s3` in S6d (D36).
- `fix::LOGICAL_NAMES` is a core -> fix edge until S4, when the fix crate
  claims its names itself and must pass `register_logical_name`'s refusal of
  grammar words the core seed bypasses (`data`).

## P4: design

### D38 - the generic event's two instants are `transunix` and `sendunix`, and the element's own names drop `curr`

**Decision.** `Event`'s instant is `transunix` - the instant the operation
really happened, its transaction time - and its technical clock is `sendunix` -
the instant the message was sent or received on the wire. `currunix` and
`recdunix` are deleted at every door in one sweep, and so is the `curr` prefix
of the element's own two names (the user's amendment): `curruuid` is `uuid`
and `currhashcode` is `hashcode` - the element's own identifier and content
code need no qualifier, where `prevuuid`, `crossuuid`, `crosshashcode` and
`srcuuids` keep the prefix that says whose they are. Nothing else is renamed:
`creaunix`, `exprunix`, `prevunix`, `snapunix`, `execunix`, `crosscode` keep
their names, and `currency` is not a `curr` name.

**`transunix`** (`i64`, nanoseconds since the Unix epoch, UTC, never absent):
when the operation really happened - the identity and order axis `currunix`
was. `time_uuid` floors it to milliseconds, `is_after` orders by it, an
`InstantSequence` run is the events at one `transunix` and `seqnum` the place
within it, `prevunix` is the predecessor's, `creaunix` defaults to it, an
expiry's is its deadline and a snapshot view's the tick (`snapunix` the
original), candles bucket by it, `BookService` and the `Lifecycle` view filter
and order by it, `IsinRegistry` dates `updunix`/`firstunix`/`lastunix` by it,
`execunix` falls back to it. FIX: the row's stated `transunix`, else
`TransactTime(60)` or a `TrdRegTimestamps(768)` stamp of an event type within
`official_time_delay_ms` (1000) of the sending clock - the venue's own answer
to when it happened - else the sending clock: `official_unix` as it is,
re-spelled. Text: the line's one instant - the `mtime` capture under
`parse_mtime`, else the handle's `mtime`, the epoch where the line has none; a
capture named `transunix` is refused as `currunix` is (`DERIVED_EVENT_COLUMNS`).

**`sendunix`** (`Option<i64>`, the same count): the technical clock - when the
message crossed the wire as the nearest clock saw it: FIX the carrier's instant
where a carrier exists (the text line's `transunix`, the capture's write
time), else the sender's `SendingTime(52)` where stated, else none - today's
`recdunix` precedence kept, because `SendingTime(52)` is already a column of
the fixed row and the capture's clock has no other; text a row-header capture
named `sendunix` alone (`EVENT_CAPTURES`), never the `mtime`. Outside identity
and every digest, as `recdunix` was. The merge reference keeps its shape over
the name: the statement sent last is the reference (`right_is_reference` - a
stated clock leads an unstated one, equal or absent clocks fall back to the
later `transunix`, exact ties keep `left`), two statements of one event keep
the earliest `sendunix` (`fold_event_instants`, the trade's `canonical_data`,
the book's `fold_bounds`), the book's `reference_clock` and the trade's
`reference_key` read `(sendunix, transunix[, uuid])`. The alternative
reading - `sendunix` the sender's `SendingTime(52)` first - is one precedence
line and its tests; it was not taken, and the handoff names it for the user.

**The element's own names.** `uuid` (`ElementColumn::Uuid`, display `UUID`,
`get_uuid`/`set_uuid`, `Event::time_uuid` unchanged as the derivation) and
`hashcode` (`ElementColumn::HashCode`, display `Hash Code`,
`get_hashcode`/`set_hashcode`) are the first two of the six element columns,
their positions, datatypes and derivations unchanged: `hashcode` is still the
XXH3-64 of what the element states and `uuid` the UUIDv7 over `transunix`,
`seqnum`, `hashcode` and `crosshashcode`. A text row's six element columns
open with `uuid`; the `marketdata` row, the FIX row, the BookService keys and
the pages spell them.

**Wire.** Crate fields 65_001 `uuid`, 65_004 `hashcode`, 65_007 `transunix`
and 65_009 `sendunix`: the tags kept, the names and descriptions re-spelled
(`UUID_TAG_NAME`, `HASHCODE_TAG_NAME`, `TRANSUNIX_TAG_NAME`,
`SENDUNIX_TAG_NAME`; every crate-field text naming one of the four re-spelled
with them), so the crate's field shard and the fixed-row component are written
again (the dump, `YGGDRYL_FIX_DUMP_WRITE=1`), the dictionary hash moves once
with the sentence "It last moved when `curruuid`, `currhashcode`, `currunix`
and `recdunix` became `uuid`, `hashcode`, `transunix` and `sendunix`: 65_001,
65_004, 65_007 and 65_009 re-spelled with their descriptions, so the crate's
field shard and the fixed row component were written again. No count of the
census below moved.", `docs/assets/fix.json` is
regenerated after the addon, and `rust/tests/fix/equivalence.snapshot` is
written again (`YGGDRYL_FIX_EQUIVALENCE_WRITE=1`): its keyed lines
re-spelled (`field.uuid`, `field.hashcode`, `field.transunix`,
`field.sendunix`), every value and every digest line unchanged - the digest
feeds no instant and no label. `EventColumn::TransUnix` and `SendUnix` keep positions 1 and 3 of
nine (display `Transaction Time`, `Sending Time`; descriptions "When the
operation happened: the settled transaction instant, UTC." and "When the
message crossed the wire, where that is known; the earliest its statements
know."). The `marketdata` row, the FIX row, the text row, the `BookService`
JSON and CSV keys and `node/book/audit.js` spell the new names. The one open
pin: `rust/src/fix/msg.rs:3201` pins a content code whose doc says a column
relabel moved every content code; the slice reads the content-code feed and
states whether the four labels enter it - if they do, that pin and every
`uuid` move once with the sentence; if not, both hold.

**Bindings and pages.** Python and Node getters, setters, the constructor's
first positional (`transunix`), `**facts` keys, error texts ("states
`transunix` once, as its first argument"), the `.pyi` stubs,
`.api-bindings.txt`, `.api-inventory.txt`; Node re-spelled only, no door
added. Every page and skill, the schema tables (tag literals unchanged),
`docs/fix/capture.md`'s dating rule, `docs/graph/serve.md`'s CSV header;
AGENTS.md: the text row contract sentence, the `graph/` (`element.rs`,
`column.rs`), `fix/` and `isin_registry.rs` rows.

**Pins.** `MarketData` at 912, every `allocations` and `iobase_calls` row and
the census unmoved; the hash, the dump, the snapshot and `fix.json` move once
each as above.

**The sweep's anchors beyond the bare names**: `get_/set_currunix`,
`get_/set_recdunix`, `get_/set_curruuid`, `get_/set_currhashcode`,
`walked_currunix`, `EventColumn::CurrUnix`/`RecdUnix`,
`ElementColumn::CurrUuid`/`CurrHashCode`, the `*_TAG_NAME` constants,
upper-case `CURRUNIX`/`CURRUUID`/`CURRHASHCODE` keys in binding tests,
`fix_event_recdunix`/`graph_order_event_recdunix`/`eventCurrunix` typing
names, test names, the Iceberg declarations in tests (`time_bucket('15
minutes', currunix)`, the sort and key lists); `refrecdunix` - the retired
FIX field 65_064, a name of its own - is excluded by the regex and not
renamed.

Slice: P4, in place, after S3 lands and before P5 (D37); one commit.

## P5: design

### D37 - `MarketMessage`, the market side's own message, and the FIX codec's doors onto it

**Decision.** `MarketMessage` is a concrete public struct in
`rust/src/graph/message.rs` - the one message every protocol lands in and the
one `MarketData` holds whole. S3's `pub trait MarketMessage`,
`MarketData::Fix(Box<dyn MarketMessage>)`, `as_message::<T>()`, `into_any`,
`clone_box` and `dyn_eq` are deleted; nothing in `graph/` names a FIX type.

**The type.**

```rust
pub struct MarketMessage {
    facts: Box<OperationEventFacts>, // every element, event, market and operation fact, typed - the one owner
    stated: StatedFacts,             // the facts the source stated, one bit each (FixMsg's 28 bits, generic)
    root: Arc<Field>,                // the protocol's entry root - one Field per dictionary, shared
    row: Scalar,                     // the entries: the ordered run under root, what was stated beyond the facts
    children: Vec<MarketMessage>,    // the elements the message states - a book message's levels - each a message
    metadata: Metadata,              // the keys no dictionary resolved
    anomalies: Vec<Anomaly>,         // what a parse or a walk could not honour
    instrument: InstrumentStatement, // countrycode, underlyingisin, eusipacode stated of its instrument
}
```

- `Element`, `Event`, `Market` and `Operation` are implemented once, on
  `MarketMessage`, through `delegate_event!`, `delegate_market!` and
  `delegate_operation!` over `facts`; `with_previous`, `merge_with` and
  `restating` are concrete over `Element`'s combinators
  (`following_operation`, `merging_operation_event`, `fold_anomalies`, all
  generic); `note_conflict` pushes an `Anomaly`; `follow_identity` is the
  trait's provided one. `OperationEventFacts` stays crate-private.
- The entries are the one row shape: `record() -> FieldRecord<'_>`,
  `entry(name) -> Option<FieldScalar<'_>>` by exact name through
  `Field::index_of`, `with_entries(root, row)` canonicalizing once under the
  root. No second schema class, no per-read parse: a cell read is one buffer
  read. A tag index is FIX's, on its handle.
- `children` are full messages - facts filled by the codec at parse, the
  record under the group's root (one `Arc<Field>` per group per dictionary);
  the parent's record holds no cell for them. `into_market_data(self)`: with
  children, one leaf per child the book records
  (`MarketDataKind::is_recorded`), the parent's facts carried into what the
  child states nothing of (`carry`, as `expand_message` does today), each
  child's facts box moved; without, one leaf by `marketdatakind`, the facts
  box moved, zero allocations; a book message with no children one scoped
  snapshot control. `into_market_leaf` as today. `expand_message`,
  `operations`, `direct_of`, `is_book_message`, `carry`, `direct_unmapped`
  and `FixMarketIterator` move from `fix/market.rs` to `graph/message.rs`
  (`MessageIterator`, the lazy projection of sorted messages into leaves) and
  read facts alone - every FIX reading they made (tag 268, `MDEntryType`,
  `MsgType`) is a fact the parse filled.
- `Anomaly { field, reason }` in `graph/anomaly.rs` is `FixAnomaly` moved and
  renamed - no row holds it today and none will.
- `InstrumentStatement { countrycode: Option<Country>, underlyingisin:
  Option<Isin>, eusipacode: Option<Eusipa> }`: what a message states of its
  instrument that is a registry fact and no element fact - set by a codec at
  parse (FIX from `UnderlyingSecurityID(309)`, `UnderlyingSymbol(311)` and the
  bridge keys exactly as `stated_underlying_isin`/`stated_eusipa` read them
  today), read by `IsinRegistry::learn_stating` through `instrument()`,
  lifted into no column.
- `StatedFacts`: the bits of the facts a source stated; `FixMsg`'s `stated`
  and `row_stated` move here, its `stale` bits vanish - the row holds no fact
  cell to go stale.
- Size: about 170 bytes inline in `MarketData::Message(MarketMessage)`, the
  facts boxed as `FixMsg` boxes them today; `size_of::<MarketData>() == 912`
  holds, `SnapshotEvent` the widest leaf.
- `MarketData::Message(MarketMessage)` replaces `Fix(..)`;
  `MarketKind::Message`, spelled `message`, replaces `Fix`/`fix` - last in
  the order as `fix` was, the `kinds` pins in both bindings and
  `rust/tests/graph/kind.rs` re-pinned with the sentence; `From<MarketMessage>
  for MarketData`, `TryFrom<MarketData> for MarketMessage`, `as_message()` and
  `as_message_mut()` borrow; `From<FixMsg> for MarketData` is `into_message`
  then hold; the `marketdata` writer and the book fold split a held message
  through `into_market_data` as they split a FIX message today.
- The message's own row form: `MarketMessage::field(root: &Field) -> Field` -
  the fact columns in `marketdata` order (`ElementColumn`, `EventColumn`,
  `MarketColumn`, `OperationColumn`), then the entry root's children, then
  `children` (`serie<struct<..>>` of the child form, one level), `metadata`,
  `anomalies` (`serie<struct<field: utf8, reason: utf8>>`) -
  `into_scalar(&self)` and `from_scalar(root, &Scalar)`; pickle and the value
  stream ride it. It is not the FIX fixed row, which is the codec's.

**FIX.**

- `FixMsg` stays as the codec's handle over a message: `registry:
  Arc<FixRegistry>`, `message: MarketMessage`, `header: Box<FixHeader>` (the
  typed header cache), the tag and name indexes into the record, the capture
  it was cut from where one exists. `FixLifted` is deleted: the record's cells
  are the typed storage and `LIFTED_TAGS` render from them. `into_message(self)
  -> MarketMessage` moves, zero allocations; `from_message(registry, message)
  -> Result<FixMsg>`: a message under this registry's entry root adopted as
  it is, its indexes rebuilt once, O(entries), no text parsed; one under
  another root re-rooted - an entry whose name the registry resolves lands
  under that field, one it does not in `metadata`, the facts untouched.
  `FixMsg`'s hand-written `Element`/`Event`/`Market`/`Operation` impls are
  deleted; the codec reads facts through `message()`/`message_mut()`, and a
  view that must be an event (the lifecycle's `LifecycleMessage`) delegates
  to the message as it does today.
- The FIX entry root: one `Arc<Field>` per registry built beside
  `fix_schema` - the fixed row's columns less the fact columns (the crate tags
  `Typed::fact` maps onto facts), less `metadata`, less the elements group
  the children render; `fixentries` among them. `into_row` interleaves the
  fact columns (from `facts`) and the entry columns (from `row`, shared) by a
  per-registry column index; `from_row` is the inverse, the entries' run
  shared; the `NoMDEntries(268)` occurrences render into `fixentries` from
  the children, byte for byte as today. The fixed row's definition and column
  order, the crate dump, `fixmsg` (65_053) and the dictionary hash are
  untouched; the equivalence snapshot is unmoved.
- A tag the dictionary's idmap maps onto a fact - `Side(54)`, `Price(44)`,
  `OrderQty(38)`, every one of them - is never an entry: it is read into the
  fact at parse and rendered from the fact at `into_row` and on the wire. The
  entries are the tags the idmap does not consume. So "a FIX message writing
  the side as `Side(54)`" is the render rule, and `follow_identity` needs no
  FIX override.
- Wire: `into_bytes`/`into_text` as today for a parsed message. For a message
  no FIX parse built - its root another dictionary's or empty - the renderer
  writes each stated fact under the standard tag the idmap names for it (one
  tag per fact, the inverse of the enrichment, the table listed in the
  contract from `Typed`/`identity::record`), else under its crate tag
  (65_0xx), so no stated fact is lost; `MsgType(35)` from `marketdatakind`
  and `state` where unstated - `8` for an order or an execution event, `S` for
  a quote, `W` for a book, `AE` for a trade, the table beside
  `fix::state::from_msgtype`; the header from the entries; the entries by
  tag in pre-order; the trailer stated only. `FixCodec::parse(render(m))`
  restates `m` - facts, stated bits, entries, metadata, children equal - the
  round-trip pin in `rust/tests/fix/`.
- `stated_underlying_isin` and `stated_eusipa` become the parse's
  `state_instrument` fill; `refill_instrument_ids` stays the parse's.

**Bindings.** Python: `MarketMessage` (frozen, hashable, pickled through its
row form), `FixMsg.into_message()`, `FixMsg.from_message(registry, message)`,
`MarketData(message)` intake, `MarketData.as_message()` replacing `as_fix()`,
`Anomaly` replacing `FixAnomaly`. Node: `asFix()` re-spelled `asMessage()`
answering a `MarketMessage` object - the class Node must carry to keep the
door it has - `MarketData`'s intake of a `FixMsg` converting inside,
`FixAnomaly` re-spelled; nothing else added.

**Cost.** `into_message` and `into_market_leaf` zero allocations (the facts
box moves); the rekey rows (`allocations.rs` @8595) unchanged; bytes per
parsed message within the @8715 pin - the caches that leave `FixMsg` pay for
the message's vectors; children exist for book messages alone, and a new
row at a `W` fixture states their cost; a new `allocations` row for
`into_message`/`from_message`; the `fix` bench gains the render of a native
message.

**Pages.** `docs/graph/market-data.md` (the message, its doors, its row
form), the FIX pages' from/into, skills `yggdryl-market-data` and
`yggdryl-fix`; AGENTS.md: the `graph/` row (`message.rs`, `anomaly.rs`,
`market_data.rs`), the `fix/` row (`FixMsg` the handle; the idmap rule), the
S4 note (the 31 market-owned items FIX reached mostly gone: a public message
with public trait doors needs no `from_facts`).

Slice: P5, in place, after P4 (it speaks `transunix`/`sendunix`) and before
S4; one commit.

## P3: design

### D36 - `yggdryl-s3`: the object-store backend as the ninth crate, through a storage-backend extension point

**Decision.** The object-store backend (`rust/src/s3/`: Amazon S3, Google
Cloud Storage, Azure Blob Storage) reaches `Holder` through a register, as a
medium reaches `Media`, and leaves for `rust/s3/` as `yggdryl-s3`. `aws/` and
`auth/` stay in the core under the `aws` feature.

1. **The register.** `rust/src/holder/backend.rs`: `StorageBackend` - `name()
   -> &'static str` (the crate), `schemes() -> &'static [Scheme]`,
   `is_property(&str) -> bool`, `holder(&Url, &[(String, String)]) ->
   Result<Holder>` - one `static` per backend, claimed all-or-none on
   `plugin::Register<Scheme, &'static dyn StorageBackend>` (`claim_backend`,
   `backend_for`, `backends`), asked by `Holder::from_url` after the
   identifier is lowered and after the local and ZIP arms, before `http`, its
   answer `described` as every other (media type, codec - the pin P3 writes
   first, missing today). A scheme a core arm holds (`file`, `zip`, `http`,
   `https`, `memory`, ...) is refused at claim. `Locator` stays what it is - an
   object a location names, asked before lowering - and is not widened. The
   ten object-store schemes (`s3 s3a s3n gs gcs az abfs abfss wasb wasbs`) are
   the backend's claim; `Holder::is_backend_property` reads
   `backend_for(scheme)?.is_property`; a scheme no claim answers is refused
   naming the crate to install.
2. **The handle.** `Holder::Registered(Box<dyn RegisteredHandle>)`,
   `RegisteredHandle: IOBase + Sync + Debug` with `implementation_name`,
   `exists(&self) -> bool` (its role's), `reopen(&self) -> Result<Holder>`
   (what `from_handle` answers: the same client, nothing sent), `as_any` and
   `as_any_mut`. `S3Folder`, `S3Path` and `S3File` leave the enum: three `cfg`
   arms out of each of the six exhaustive matches, one `Registered` arm in.
3. **The capabilities** the wildcards specialized on S3 become `IOBase`
   methods with defaults - no second storage trait: `upload_from(&mut self,
   source: &mut dyn Read, length: u64) -> Result<()>` (default: the source
   read whole then `write_all_bytes`, today's fall-through), `discard(&self)
   -> Result<bool>` (default `false`: nothing staged to drop, the caller
   removes), `as_leaf(&self) -> Result<Option<Holder>>` and
   `as_container(&self) -> Result<Option<Holder>>` (default `None`: the handle
   as it is), `set_known_size(&mut self, size: u64)` (default no-op);
   `into_byte_stream`'s `Registered` arm reads the existing
   `owned_stream_bytes` (S3's lazy resuming `GET`, one request) else the
   cursor reader. `delegate_iobase!` forwards them, so every wrapper keeps
   them. `iceberg/staging.rs` (`upload`, `unpublished`, `leaf`, `container`,
   `sized`), `iceberg/catalog/mod.rs` `folder_role` and Python's `Role::of`
   lose their S3 arms and call the verbs. Every request-count pin holds: the
   trio keeps its own `IOBase` impls, and construction, child resolution and
   a reopen stay request-free.
4. **`Site::Store`** becomes `Site::Opened { url: Url, open: Arc<dyn
   Fn(&Properties) -> Result<Holder> + Send + Sync> }` - `Eq`/`Hash` by url,
   `Debug` by hand - built by `s3tables/` over the backend under the bucket's
   session; `warehouse/handle.rs` names no AWS or S3 type.
5. **`aws/` and `auth/` stay core**, under `aws`: who this process is to AWS
   for every service, `with_sigv4` an inherent method of `http::Request`,
   `ureq` in no published signature. The 58 non-public items `s3/` reaches
   land in `implementer` (gated as their owners are, the ureq-typed
   forwarders included - the door is hidden); the 23 `cfg(feature = "s3")`
   sites in `aws/`, `auth/`, `http/` and `xml/` re-key to the feature of the
   module that holds them.
6. **Edges.** D15 amended: `yggdryl-iceberg`'s `s3tables` depends on
   `yggdryl-s3` (which implies `yggdryl[aws]`) and installs it; the core's `s3`
   feature is deleted at S6d (P3 keeps it in place); the bindings and the CLI
   link the crate and install it at import and start; a pure-Rust
   `Holder::from_url("s3://..")` before `install()` is refused naming
   `yggdryl-s3`, as a medium is.
7. **Pins.** The 11 core-resident pins over S3 handles and the five
   `accounting::iceberg` tests move at S6d to `rust/iceberg/tests/` (a
   dev-dependency on the leaf from the core's tests is a cycle; from
   iceberg's it is the `s3tables` edge); `FakeS3` stays in
   `rust/tests/support/`, `#[path]`-included by the crate's tests; the three
   exchange scripts keep their `--test interop s3::<dialect>::` and move with
   the tests.
8. **Bindings.** Python's `S3File`/`S3Folder`/`S3Path` classes pick by
   `downcast_ref` through `as_any` (the `warehouse.rs` precedent), `cloned`
   replaced by `from_handle`; Node's `folder_holder_for` links the crate.
9. **CI.** `[leaves]` gains `s3 = { package = "yggdryl-s3", jobs =
   ["object-interop", "azure-interop", "gcs-interop"] }` and `iceberg` takes
   `after = ["s3"]`; the planner's `LEAF_LINE` reads `[a-z][a-z0-9]*`,
   `EXCHANGES` gains the three, `NEVER` drops them for `s3`, its tests with
   it.
10. **Slices.** P3 in place: the register, `Holder::Registered`, the `IOBase`
    capabilities, `Site::Opened`, the trio claimed by the core itself under
    `CORE` at startup, the missing `from_url` pin, the pages (the Object
    stores section of `docs/holder/index.md` gains the extension point); S6d
    the move to `rust/s3/` with `install()`, the leaf line, the tests moved.
    P3's files are disjoint from P4's and P5's (`holder/`, `iobase.rs`,
    `iceberg/staging.rs`, `iceberg/catalog/mod.rs`, `warehouse/handle.rs`,
    `s3tables/`, `s3/`, the bindings' handle files), so it runs beside them
    and lands when its chain is clean.

Names: `StorageBackend`, `RegisteredHandle`, `Holder::Registered`,
`claim_backend`, `backend_for`, `backends`, `Site::Opened`.

## S6: design

### D39 - a leaving medium keeps its rank, and what every S6 move does the same way

**The rank.** `RecordOptions` hashes and orders by the codec's rank, so the
`s2_pins` hashes and the order pin are the rank's. `media::codec` gains
`RESERVED_RANKS: [(&str, u8); 4] = [("parquet", 1), ("avro", 2), ("xmla", 4),
("excel", 6)]` - the ranks the core's leaving media hold, a wire contract that
never moves, the shape of `MarketDescriptor::RESERVED_*` - and `claim` admits a
codec whose `(name, rank)` is one of those pairs, refuses a reserved rank under
another name or a reserved name at another rank, and holds every other medium
at or above `EXTERNAL_RANK`; the `CORE` refusal stays. So `yggdryl-parquet`'s
`install()` claims rank 1 as the core did, and every pinned hash, the order pin
(`the_media_order_by_their_rank`, `media_register.rs`'s expected order) and
the rank-1 assertion on the media page are byte-identical through the moves.

**The door.** `implementer.rs` grows once for every media move - the items the
four maps list (`moves_map/{avro,parquet,excel,xmla,iceberg}.md`, section 2:
about 18 for Avro, 15 for Parquet, 24 for Excel, Iceberg's 46 path items and
the methods reached by call) - by S3's routes: raise-and-re-export inside a
crate-private module, a forwarder, a free function over a `pub(crate)`
inherent method, a move, an exported macro. The eight shared medium helpers
(`iobase::transfer::{overwrite_arrow_reader_default_with_field,
append_arrow_reader_default, merge_arrow_reader_default, leaf_writer}`,
`iomedia::{own_options, container_field, container_row_size,
dimension_options}`) are one addition serving every move. What Iceberg
reaches of Avro (the container header, `Cursor`, `DatumCodec`, `parse_header*`,
`MAGIC`, `Blocks::metadata_bytes`) and of Parquet (`ParquetOptions`,
`read_batch_reader_with`, `load_metadata`, `schema_from_metadata`,
`overwrite_buffered`, `READ_AHEAD_BATCHES`, `WHOLE_READ_BYTES`,
`WRITE_BUFFER_BYTES`, `FileStatistics`) is `pub` in those crates under a
`#[doc(hidden)] pub mod implementer` of their own, the same door one level
down.

**The Iceberg view.** `Field::as_iceberg`/`as_iceberg_mut` and the inherent
`impl IcebergField<'_>` leave the core as the FIX view did (D9):
`protocol_field_types!` builds `IcebergField`/`IcebergFieldMut` in
`yggdryl-iceberg`, `IcebergField::new(&field)` the one spelling, the sites
(about a hundred, in tests, benchmarks, pages and skills) swept by one script.
`Transform` and the Iceberg type-string spelling (`iceberg/types.rs`,
`PrimitiveType`) stay the core's (D17).

**The tests.** A core test that builds a leaving crate's object (an
`IcebergTable` in `isin_registry/store.rs`, `warehouse/*`, `graph/serve.rs`,
`fix/schema.rs`, `s3/mod_.rs`; an Avro or Parquet handle in about fifteen
files) cannot dev-depend on that crate (a cycle), so it moves to the crate's
own `rust/<name>/tests/`, its `//!` line naming the core file it pins and the
crate it needs; `tests/allocations.rs`' Iceberg block moves with its own
counting allocator; `FakeS3` and `support/excel_package.rs` stay under
`rust/tests/support/` and are `#[path]`-included where used.

**`install()` at init.** The Python `_native` module init, the Node addon init
and the CLI's `main` call `yggdryl_<crate>::install()` once per linked crate,
in dependency order; a pure-Rust caller installs what it links, and a handle
whose medium no crate claimed is refused naming the crate, as today. The
core's seed claims (`media/codec.rs`, `media/format.rs`,
`warehouse/catalog.rs`, `holder/locator.rs`) go with each move. Avro's
`snap` and the `snappy` gates leave the core's `parquet` feature with the
crate (D16); the dead `cfg(iceberg|s3tables)` arms in the core are deleted.

**The order.** Inside lane M46 after S4: avro, parquet, excel, xmla - each a
commit, one chain and one push for the batch - then, after S6d has landed
`yggdryl-s3`, iceberg with `s3tables` as its feature depending on
`yggdryl-s3` (D15, D36). `iceberg` is `after = ["avro", "parquet", "s3"]` in
the CI leaf table.

## The ledger

| D# | decision | evidence | slice |
| --- | --- | --- | --- |
| D1 | program branch `ccr-0fe6f9d0-ruymat` (harness-assigned); draft PR; merge `origin/main` per session; version `0.1.21` until S9 | the harness's branch rule; `ci.yml:3-9` | S0 |
| D2 | closed shape over `&'static MarketKind` descriptors; `Scalar` 48 and `Serie` 40 bytes | the spike; `allocations.rs:6540` | S0, built S1 |
| D3 | `DataTypeId(u8)` newtype with CamelCase consts; the four enum kinds' ids leave it and the seventeen codes' stay (D25); `all()` answers the claimed order | the spike; `datatype_id.rs:28-32, 270` | S0, built S1 |
| D4 | `State` and the event vocabulary stay core; `State`'s FIX tables move to `fix/` in S3 | `text/line.rs:11-12`, `text/plan.rs:20` | S0 |
| D5 | the market type's own `dtype()`/`field(name)` items; no extension trait | E0116 | S3, built S3 |
| D6 | one public `implementer` module of forwarders, per-slice list from a scratch `git mv` | AGENTS "never make an item pub inside a published module" | S1, S3, S6; S3's list built S3 |
| D7 | explicit idempotent `install()`; intake alone reads the register; the refusal list; the core seeds itself until S4 | `lib.rs:11` denies `unsafe`; `datatype_kind.rs:140-178` | S0, built S1 |
| D8 | `plugin.rs`: one claim-once `Register<K, V>` and the `Dyn*` helper; market, media and `LOGICAL_NAMES` keys on it | `expression/user.rs:483-494` | S0, built S1/S2 |
| D9 | a market trait for a message that splits; `MarketKind::Fix` stays; a protocol-view builder | `graph/market_data.rs:44`, `protocol.rs:2078` | S3, built S3 |
| D10 | the table above | `scheme.rs:27-420`, `mime_type/line.rs:80-100`, `logging/facade.rs:72-76`, the 59 `warned!` and about 120 `log` sites counted there | S3, built S3 |
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
| D21 | one trait per role on the register; the pushdown rides the options; refined by D26-D31 (the whole options struct behind the box, `open(handle) -> Media`, `as_any` answering a state object) | `media/options.rs:142, 1343`; `iceberg/mod.rs:146-318` | S0, built S2 |
| D22 | `pluginside` deleted; the plugin's role is a `Side` read by `fix::plugin_side`; `buyside`/`sellside` spell `BUYS`/`SELL`; byte `0xc6`, `Shape` 72 and rank 32 retired; the dump, the codeset and the dictionary hash moved with it | the user's instruction; `side.rs`, `fix/source.rs`, `fix/crated.rs` | P0 |
| D23 | the medium holds its `RecordOptions`; the media serie keeps no copy | the user's instruction; `media_serie.rs:18`; the understanding workflow's map | P1 |
| D24 | `yggdryl-excel`, a seventh crate through the media point, claimed in S2, its own commit after S6; the bindings keep their Excel doors in the one native module | the user's instruction; the 39 core sites and the import lines above; crates.io 404 | S0, built S6b |
| D26 | `RecordOptions::Registered(RegisteredOptions)`, a `Box<dyn MediumOptions>` holding the medium's whole options struct; `Parquet`/`Avro`/`Excel` variants and accessors deleted; `Ord`/`Hash` by the codec's rank; the typed `settings` door | the S2 pins; `media/options.rs:1344-1360`, `dispatch.rs` | S2 |
| D27 | `MediaCodec` statics claimed on the register, `RecordOptions::codec()` the one dispatcher; `Media::Registered(Box<dyn MediaWrapper>)`; `codec_for` the one refusal | `transfer.rs:1171-1323`, `media/mod.rs:117-159`, `holder/mod.rs:771-819` | S2 |
| D28 | `Serie::{Parquet, Avro, Excel, IcebergTable}` deleted, `GenericMediaSerie` serves them; `require_kind` by MIME set | `media_serie.rs:510-533`; the eleven invocations | S2 |
| D29 | `TableFormat`/`LocatedTable` on the register, `ReplacedPartitions` inside the located table; the layout detection stays core | `iceberg/mod.rs:146-458`; the eleven `located` sites | S2 |
| D30 | `Catalog`/`Namespace`/`Table::Registered`; `CatalogFactory` by type word and scheme; `Locator` by scheme; `Site::Store` under `s3` | `warehouse/catalog.rs:72-143`, `holder/mod.rs:348-451`, `handle.rs:19-47` | S2 |
| D31 | `Error::External { origin, reason, source }`; `From<ParquetError>` deleted for `map_err`; `IOMedia::as_any` answering the Parquet footer cache; the statistics as free functions over `&dyn IOMedia` | `error.rs:166-172`, `parquet/mod.rs:2524`, `iomedia.rs:369-411` | S2 |
| D32 | `filter_phases` published; logging and D17 unchanged | `expression/mod.rs:1036`; `logging/facade.rs:72-76` | S2 |
| D34 | the medium holds its origin (`read_origin_field`), one schema answer, one projection rule (declared ∩ the columns the select and early filter read), one composer in `media_serie.rs` that `read_record_serie` also calls, `source_field` and the five residual copies gone; S8 amended | the user's instruction; `pushdown_map/design_inputs.md`; "P2 results" | S2b, built P2 |
| D35 | `MediaCache` on every wrapper under `cache_ttl` (milliseconds, 0 realtime, outside the hash feed as `file_threads`), served while open or younger than the TTL, every write door updating or invalidating it | the user's instruction; `pushdown_map/metadata_caches.md`; "P2 results" | S2b, built P2 |
| D33 | `yggdryl-xmla`, an eighth crate through the media point, `soap/` with it; registered in place in S2b (the core's own media three: `RecordOptions::Xmla`, `Media::Xmla`, `Serie::Xmla` deleted), moved in S6c; the Python `Xmla` class stays in the one native module; the CLI's `xmla serve` depends on it | the user's instruction; the 32 core sites and the 26 import lines above; crates.io 404 | S2b, built S6c |
| D39 | a leaving medium keeps its rank: `media::codec::RESERVED_RANKS` (`parquet` 1, `avro` 2, `xmla` 4, `excel` 6) admitted by `claim` under the codec's own name, every other medium at or above `EXTERNAL_RANK`, so the `s2_pins` hashes and the order pins are byte-identical through S6; `implementer` grows once per move by S3's routes, Avro and Parquet carry their own hidden `implementer` for what Iceberg reaches; the Iceberg field view built by `protocol_field_types!` in `yggdryl-iceberg` (`IcebergField::new`), `as_iceberg` gone; core tests building a leaving crate's objects move to that crate's tests; `install()` at every init; order avro, parquet, excel, xmla, then iceberg after `yggdryl-s3` | the five media maps | S6 |
| D38 | `currunix` -> `transunix` (the transaction instant, required, the identity and order axis), `recdunix` -> `sendunix` (the technical wire clock, optional, the merge reference), and the element's own `curruuid` -> `uuid`, `currhashcode` -> `hashcode` (`prevuuid`, `crossuuid`, `crosshashcode`, `srcuuids` keep their prefix); precedences, values, derivations, positions and tags unchanged - the carrier's clock first, else `SendingTime(52)`; crate fields 65_001, 65_004, 65_007 and 65_009 re-spelled, so the dump, the dictionary hash (once, with its sentence), the snapshot's keys and `fix.json` move and the census does not | the instants map | P4 |
| D37 | `MarketMessage` a concrete public struct in `graph/message.rs` - boxed facts, `StatedFacts`, the entries as an `Arc<Field>` root and a `Scalar` row, `children`, `Metadata`, `Vec<Anomaly>`, `InstrumentStatement` - the four traits implemented once on it; `MarketData::Message`, `MarketKind::Message` (`message`); `FixMsg` the codec's handle over a message (`into_message`, `from_message`), an idmap-mapped tag never an entry, a native message rendered by the inverse idmap else its crate tags; S3's trait, `as_message::<T>()` and `Box<dyn MarketMessage>` deleted | the message map, `message_map/design_inputs.md` | P5 |
| D36 | `yggdryl-s3` through a storage-backend extension point: `StorageBackend` claimed per scheme on the register (`claim_backend`, `backend_for`, `backends`), asked by `Holder::from_url` after lowering, its answer described; `Holder::Registered(Box<dyn RegisteredHandle>)`; the verbs the wildcards specialized on S3 (`upload_from`, `discard`, `as_leaf`, `as_container`, `set_known_size`) as `IOBase` defaults and `into_byte_stream` over `owned_stream_bytes`; `Site::Opened` with an opener; `aws/` and `auth/` stay core under `aws`; `yggdryl-iceberg[s3tables]` depends on `yggdryl-s3`; CI leaf `s3` with the three exchanges | the backend map, `s3_backend_map/design_inputs.md` | P3 in place, S6d the move |
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

## The CI structure

The user's instruction: "Ensure then ci, builds are first core and then
parallelized, add then exclusion rules strategy to only build leaves crates
projects to implement faster on leaves implementations". Before it, 17 jobs
ran everything on every push; the wall clock was the serial all-features
lane, 18m09s (runs 37842429776 and 37839852909: 18m11s and 18m28s) - 2m02
clippy, 13m10 tests of which 3m58 compiled and about 9m12 ran the 71
targets one after another (`tests/fix.rs` 184 s, `tests/allocations.rs`
116 s) - and the core was compiled again in 14 of the 17 jobs.

Chosen: the second judge's synthesis of three designs - core lanes compiled
once per feature set inside the run and handed on as artifacts, path rows
plus a proof ledger, one gate - as implemented with two changes of the
implementer's: a lane carries the core library and the `interop` target,
never the test binaries (about 14 GB a lane), so each shard compiles its own
targets once; and the planner is a standard-library script with its own
tests (`scripts/ci/plan.py`, `scripts/tests/test_ci_plan.py`) rather than
`dorny/paths-filter`, so classification and fingerprints read one glob
grammar. The review's seven findings, as decided:

| # | Finding | Disposition |
| --- | --- | --- |
| 1 | adding a leaf turned CI red: the tests hard-coded the commented lines | fixed: the tests read every `[leaves]` line, listed or not, and derive their expectations from the table; proven with `market` and `fix`, then all seven, switched on over stub manifests |
| 2 | a leaf-only change ran nearly every binding and docs job | fixed: `libs` no longer extends the leaves; a leaf row runs the `Leaf` matrix, `[leaf] jobs` and its line's `jobs`, the `python` job one leg (`python_legs`) |
| 3 | jobs restoring no lane waited for `core-default` | fixed: `cli`, `node-addon`, `docs-rust` and `leaves` need the plan alone; the barrier is gone; one dependency cache per leaf, saved on `main` |
| 4 | `retention-days: 1` broke a next-day re-run | fixed: three days on every artifact, pinned by a test |
| 5 | deleting `order.toml` failed the planner | fixed: an unknown path runs everything, pinned; nothing asserts it is tracked |
| 6 | the Python lane built PyO3 without maturin's `abi3` | fixed: `core-python` folded into `python-wheel`; the free-threaded job builds its own |
| 7 | docs said more than the YAML | fixed: the contributing and playground pages and AGENTS.md's ledger sentence |

The leaf rule - a change under `rust/<leaf>/`, its manifest aside, for the
leaf and every leaf `after` it:

| Runs | Skips |
| --- | --- |
| the `Leaf` matrix (clippy and rustdoc on its package, its tests and rustdoc examples in both lanes, its MSRV check); `fmt`; `inventory`; `docs-rust`; `python-wheel` and the `pyarrow>=18` leg; `node-addon`, `node` and the `cli` it spawns; its exchanges | the core test shards, both lints, `python-freethreaded`, the `pyarrow==18.*` leg, `docs-python`, `docs-javascript`, every other exchange |

| Leaf | Its exchanges |
| --- | --- |
| `avro` | `avro-interop` |
| `parquet` | `pyiceberg-interop`, `spark-interop` |
| `iceberg` | `pyiceberg-interop`, `spark-interop`, `iceberg-msrv` |
| `excel` | `excel-interop` |
| `market`, `fix`, `xmla` | none |

A change to the core, the lock or any manifest, a path no row names, the
workflow, the table or the planner, and every push to `main`, run
everything.

Measured on its first run, 37904365402 on `1f909739b` - a workflow change,
so every row ran: 10m41s wall against 18m09s, 34 jobs of which 33 ran green
and the empty `Leaf` matrix was skipped as planned. The critical path is now
the wheel and the Python page examples (`Changes` 12s, `Python binding
wheel` 5m12s, `Documentation examples (Python)` 4m57s, `CI result` 9s);
the core's is `Core build (all features)` 1m40s then the slowest shard,
all features `rest`, 5m37s (`fix` 4m56s, `allocations` 3m23s; default
features `fix` 3m47s, `rest` 3m44s). The exchanges took 30s to 1m18s,
their cargo finishing in under a second over the lane, where each spent
about 80s compiling before; the free-threaded build, 7m56s, runs beside
the critical path. The gate proved 20 rows into the ledger.
