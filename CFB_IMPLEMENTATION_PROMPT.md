# Ullink CBlock (`.cfb`) → FIX definitions

Parse an Ullink CBlock configuration read through an `IOBase` handle and
answer the FIX field definitions it declares. Follow `AGENTS.md`.

**No paths anywhere.** The tree was refactored after this was written;
everything is named by what it does and found by symbol.

## What is landed

The FIX phase this builds on is merged. Reuse it unchanged.

| notion | what it is |
| --- | --- |
| `FixBranch`, `FixId`, `FixKey` | one field's identity and the three ways a caller names one |
| `FixField` / `FixFieldMut`, `FixAliases` | the borrowed views over the `fix:` namespace |
| `FixRegistry` | tiered resolution over a primitive and a nested half, with `insert`, `update` |
| its store | `from_handle` / `write_into` over one folder handle |
| the process-wide default registry | what a caller gets when it names none |
| `FixMsg` | a value plus the registry that types it |
| `DataType`, `LOGICAL_NAMES` | the schema grammar and the FIX Latest datatype table it falls back to |
| the committed dictionary | the seed registry, read through the store |

Nesting needs no second type: a component is a Struct, a repeating group a
List of an `item` Struct whose `fix:tag` is the counter's.

The FIX versioning brief (`prompts/fix-versioning/`) defines `FixBranchInfo`,
which this parser fills — see its rule `P2-R9b`.

## Scope

Read `vocabulary` and `grammar-binding`. Nothing else.

Add nothing beside the one constructor: no `cfb:` protocol, no `Scheme`
variant, no `MediaType`, no reserved metadata key, no document or schema type
beside `Field`. What a `.cfb` declares that the core cannot express is
dropped, and the module docs name every loss.

Skip the whole `normalization-binding` layer: message transcoding, not
schema, and a later phase. Do not sketch an evaluator for its expression
language.

## Entry point

Two facts of the landed code decide the signature:

1. `FixField<'field>` is a macro-generated newtype over
   `ProtocolField<'field>` wrapping a `&'field Field`. It cannot own a parse
   result, and an associated constructor ignoring its own lifetime does not
   belong on it.
2. `FixRegistry::insert` refuses a field carrying no `fix:tag` — "a field
   enters only with a `fix:tag`". A message root is a Struct named by a
   MsgType and has no tag, so **roots cannot enter a registry**. Do not
   invent a synthetic tag, a branch cut from `fix-version`, or a second
   keying scheme.

So the name stays and the constructor moves to the type that already owns a
handle constructor and owns fields:

```rust
impl FixRegistry {
    /// Parses an Ullink CBlock configuration into the vocabulary it declares
    /// and the message roots its grammar bindings describe.
    ///
    /// `branch` names the dialect, because a `.cfb` never names itself;
    /// `None` reads it into the standard branch. The root element's
    /// `fix-version`, `sendercompid` and `targetcompid` become that branch's
    /// `FixBranchInfo`.
    pub fn from_cfb(
        handle: &dyn IOBase,
        branch: Option<&FixBranch>,
    ) -> Result<(Self, Vec<Field>)>;
}
```

The registry half holds the `vocabulary` fields, each with its `fix:tag`,
insertable and writable. The `Vec<Field>` holds the message roots in binding
order. The constructor takes no seed registry: it answers what one file says,
and merging is the caller's business.

Read the file the way the store reads a shard.

## The format

Every fact below comes from 81 production files (133 MB, largest 6.3 MB and
300k elements; a representative one is 1'522'455 bytes over 31'587 lines).
All are well-formed `<?xml version="1.0" encoding="US-ASCII"?>`,
tab-indented, Unix LF. You will not have them: build fixtures by hand from
this description and the verbatim extract below.

### The root is the branch record

`cplugin-configuration[type version fix-version date? logs? targetcompid?
sendercompid?]`.

**Three of those attributes are the branch record, and reading them is the
first thing the parser does.** A `.cfb` is one counterparty's dictionary and
states at the top exactly what a dialect declares: `fix-version` (e.g. `4.4`)
is the FIX version it speaks, and `sendercompid` / `targetcompid` name its
session. That is `FixBranchInfo` (`P2-R9b`).

- `fix-version` parses through the FIX layer's version mapping into a
  `Version`. It never becomes a *branch*: a branch must start with an ASCII
  letter, so `4.4` is not one. Bundling version and identity on one record is
  what ends that confusion.
- `sendercompid` / `targetcompid` are stored as they arrive, compared folded.
  **`type` says which way the pair points:** its `BuySide`/`SellSide` portion
  gives the plugin's role, so the declared pair is the session seen from
  *this* side and the counterparty sees it reversed. That is why a reader
  matching a session tries both orders. Only the role is kept; the Java class
  name is not.
- `version` (the CBlock format's own, `1.2`), `date` and `logs` stay dropped.
  They describe the file, not the dialect.

**The branch name is not in the file.** The caller supplies it; absent, the
parse lands in the standard branch and declares no session — right for a file
read only for its vocabulary.

### Children and skipping

In document order: `history`, `cvs-revision`, `description`,
`message-types`, `inbound-message-type-mappings`,
`outbound-message-type-mappings`, `vocabulary`, `grammar-binding`,
`normalization-binding`, `reject-binding?`, `flow-filter-binding?`, `maps`,
`options?`, `noe-normalization-binding*`.

Skipped siblings are routinely self-closing (`<inbound-message-type-mappings />`,
`<reject-binding />`, `<flow-filters flowType="outbound" />`) and routinely
deep and text-bearing (`history/version` carries multi-line text with
entities). Skip by depth counting — never by assuming an empty element or a
known child set.

### `vocabulary`

`vocabulary-tag[name alt? type read-only? ref?]` with an optional
`description` child.

- `name` is the decimal tag.
- `alt` is the FIX field name, absent in a handful — fall back to the tag
  rendered as text.
- `ref` names the data tag this one is the length of: a Len/data pair whose
  length tag carries no independent value. Dropped.
- `read-only` is often absent on custom high-numbered tags.
- The `description` child is sometimes absent, sometimes present but empty
  (`<description />`). Its text carries entities (`&lt;SOH&gt;`, `&quot;`)
  that must be unescaped — never treat descriptions as opaque bytes.
- `type` is exactly one of `string`, `char`, `integer`, `float`, `boolean`,
  `utc-date`, `utc-timestamp`, `utc-time-only`. Anything else is a typed
  error.

### `message-types`

`message-type[value description rejection supported]`. `value` is a MsgType
optionally followed by a space and a qualifier — observed: `Inbound`,
`Outbound`, `Report`, `Report Ack`, `SD`, `SL`, `SDR`, `SLR`, `4.2`. Treat
the whole string as an opaque key; never split on the space to recover a
direction. The same composite keys reappear verbatim as `entry` values in
`outbound-message-type-mappings` (`value="P Report Ack"`), which confirms
they are keys. `supported="false"` is the common case (2231 of 2988). The
element is skipped; the rule matters because `grammar-binding/@type` carries
the same strings.

### `grammar-binding`

`grammar-binding[type merge-mode?]` holds exactly one `grammar`. Its `type`
is a MsgType such as `7`, usually equal to a `message-type` value but not
necessarily, and bindings exist for unsupported types — require neither.

`grammar[checkordering? activated? rg-name? merge-mode?]` holds interleaved
`tag-constraint*` and nested `grammar*`, to observed depth 5.

`grammar` is the only container and the only recursion point. Nested
`grammar`s are siblings as often as children: one routinely holds several,
separated by plain `tag-constraint`s. So the parser is one recursive function
over `grammar`, never a two-level special case.

A nested `grammar` is a repeating group whose first child is the counter
tag's `tag-constraint`. `rg-name`, when present, names that same tag, but it
is absent from most nested groups: first-`tag-constraint` is the hot path,
not the fallback. Resolution order is `rg-name`, else the first
`tag-constraint`, else skip the group with a typed error.

There is no component element and no reference mechanism. FIX component
blocks — `Instrument`, `UnderlyingInstrument`, `LegInstrument` — are
flattened into plain sibling `tag-constraint`s at the point of use, repeated
in full in every `grammar` that needs them. Do not recover components, factor
repeated runs into shared sub-Structs, or dedupe two grammars carrying the
same tag sequence. Only repeating groups are structure, and every one is a
`grammar`.

Three real structural exceptions exist in the corpus: an empty `grammar`; a
`grammar` whose first child is another `grammar`; and one file where
`rg-name` disagrees with the first `tag-constraint`.

### `tag-constraint`

`tag-constraint[name part required? activated? read-only? merge-mode?]`.

- `name` is the decimal tag, and always resolves in the same file's
  `vocabulary` — no dangling reference exists in any of the 81 files, so a
  miss is a genuine typed error.
- `part` is `header`, `body` or `trailer`.
- Everything but `name` and `part` is optional. Define and document each
  default rather than assuming `false`.
- `required` is usually `true`/`false`, but sometimes a condition expression
  such as `$59 = '6' and empty($126)` or `$167 = "FXSPOT" or $167 = "FXFWD"`.
  Parse it as boolean-or-expression, treat any expression as not-required,
  and never fail on it. The attribute is entity-escaped, so it must be
  unescaped before it can even be tested against `true`/`false`.

A `tag-constraint` has zero or one validity child — absent in 81382 of 301372
constraints, meaning unconstrained — plus an optional `condition` holding
`expression[value]`. One file pairs two validity elements, so tolerate more
than one.

Validity elements: `string-validity[regexp domain]`,
`char-validity[regexp domain]`, `integer-validity[domain]` over one or more
`integer-range[min max]`, `float-validity[domain]` over
`float-range[min max]`, `boolean-validity[value domain]` whose `value` is the
Java literal `BooleanValidity.UNDEFINED` or `BooleanValidity.TRUE`, and the
always-empty `utc-date-validity`, `utc-timestamp-validity`,
`utc-time-only-validity`. `domain` is `all-values` or `ranges`; anything else
is a typed error. `min`/`max` are a decimal literal or the sentinel
`minimum`/`maximum`, and both sentinels occur in one element
(`min="minimum" max="maximum"` under `domain="all-values"`), so an
`all-values` integer validity still carries a range child and must not be
special-cased into an empty one.

All of it is dropped. It is specified so the parser can consume it without
guessing, and so the docs can name the loss exactly.

### `merge-mode`

Valued `assert`, `update`, or `update|ignore`; appears on `vocabulary-tag`,
`tag-constraint`, `grammar`, `grammar-binding`, `normalization-binding` and
`map` in two overlay files patching a base configuration. Overlay merging is
out of scope: parse it, ignore it, and document that a `.cfb` carrying it is
read standalone.

## Type resolution

Resolve `vocabulary-tag/@type` through `DataType::from_str`. Logical names
fold to lowercase with `_`, `-` and space removed, so seven of eight resolve:

| `.cfb` type | resolves through | to |
| --- | --- | --- |
| `string`, `char` | the grammar's text spellings | `utf8` |
| `integer` | the grammar's integer spellings | `int32` |
| `float` | the grammar's float spellings | `float32` |
| `boolean` | the grammar's boolean spellings | `bool` |
| `utc-timestamp` | `LOGICAL_NAMES["utctimestamp"]` | `timestamp(ns,"UTC")` |
| `utc-time-only` | `LOGICAL_NAMES["utctimeonly"]` | `time64(ns)` |
| `utc-date` | **nothing** | — |

Three decisions, each explicit:

1. **`utc-date` does not resolve,** because it folds to `utcdate` and the
   table spells that name `utcdateonly`. Add the alias generically, not
   privately here — three edits pinned by one verbatim-comparison test: the
   logical-name table gains `("utcdate", DataType::Date32)` beside
   `utcdateonly`; its mirror in the datatype tests gains the same entry; the
   datatype page's `UTCDateOnly` row gains the second spelling, the way
   `Exchange`, `mic` already share one. The name is not one the Arrow/SQL
   grammar owns, so this respects the logical-name registry's rule.
2. **`float` stays `float32` and `integer` stays `int32`.** They are the
   grammar's generic answers and the wrong shape for FIX money and sequence
   numbers — where the table already spells `price` and `qty` as
   `decimal64(18,8)` and `seqnum` as `int64` — but a `.cfb` says nothing
   about which tag is money, and the second pass never invents a type. Do not
   promote by tag or consult the seeded registry mid-parse. Document it as
   the phase's principal known loss, and the consequence: merging a `.cfb`
   vocabulary into a registry seeded from the committed dictionary
   *replaces* tag 6 `AvgPx` `decimal64(18,8)` with `float32`, because
   `insert` replaces wholesale on an identity match.
3. **Nothing is inferred from validity children.** A `regexp` that pins a
   length does not become `ascii(n)`; a `domain="ranges"` does not become an
   enum.

## The mapping

Two passes over one file: `vocabulary` builds the dictionary,
`grammar-binding` builds the message trees. The second pass never invents a
type.

### Vocabulary pass

A `vocabulary-tag` becomes a `Field`: named by `alt`, else the decimal tag as
text; carrying the `DataType` its `type` resolves to; `fix:tag` from `name`;
`fix:description` from the `description` child **only when present and
non-empty** — `<description />` contributes no key rather than an empty one;
nullable, because a `vocabulary-tag` says nothing about presence.

Dictionary entries are immutable. A grammar never edits one: it clones and
overrides the clone's nullability, so a tag bound `required="true"` in one
message and `required="false"` in another yields two independent `Field`s off
one entry.

### Grammar pass

A `grammar-binding` becomes exactly one root `Field`: a non-null Struct named
by its `type` verbatim, spaces included, so `7` and `P Report Ack` are both
legal field names. The MsgType is carried by the name alone — no reserved
metadata key holds it, and the composite qualifier is never split.

The root's children are the `grammar`'s children in document order,
**flattened across `part`**: a `tag-constraint` becomes a leaf, a nested
`grammar` a repeating-group field, interleaved exactly as the file
interleaves them. Do not regroup by `part` into header/body/trailer
sub-Structs and do not sort by tag. Document order is the schema order and
`part` is dropped, so a `header` constraint and a `body` constraint sit as
siblings in one flat Struct.

**A leaf `tag-constraint`** resolves its `name` in the same file's
`vocabulary`, clones that `Field`, and sets `nullable = !required` — an
expression-valued `required` counting as nullable. The leaf keeps the
dictionary's name, `DataType`, `fix:tag` and `fix:description`; the
constraint's own validity child contributes nothing.

**A nested `grammar`** becomes one repeating-group `Field` whose `DataType`
is a List of non-null Struct. Its first `tag-constraint` is the counter and
is consumed, not emitted: the group field takes that counter's name,
`fix:tag` and `fix:description` from its dictionary entry, and
`nullable = !counter.required`, while the counter's own integer `DataType` is
discarded because the list length already carries it. The List's item field
is named `item`, is non-null, and is a Struct whose children are the nested
`grammar`'s remaining children — every `tag-constraint` after the first and
every deeper `grammar` — in document order, by the same rules recursively.
Observed nesting reaches depth 5: recurse, and raise a typed error past a
documented guard depth rather than overflow the stack.

**A duplicate tag at one Struct level** is possible now that `part` is
dropped and a tag can be bound in both `header` and `body`. Not an error and
not a deduplication: keep both children in document order and disambiguate
the later names with a documented suffix, leaving `fix:tag` identical so the
tag stays recoverable.

**The structural exceptions:** a root `grammar` with no children becomes an
empty non-null Struct rather than a failure; a nested `grammar` that is empty
or whose first child is another `grammar` has no counter to name it, so it is
skipped with a typed error naming the byte position while the parent keeps
its remaining children.

### Dropped, by name

The module docs list these as known losses: `part`, `activated`,
`read-only`, `ref`, `checkordering`, `rg-name` beyond group naming,
`condition` and every `expression`, `regexp`, `domain`, every range and
sentinel, every validity element, `merge-mode`, `message-types` and both
mapping tables, `history`, `cvs-revision`, `description` on the root,
`normalization-binding`, `reject-binding`, `flow-filter-binding`, `maps`,
`options`, `noe-normalization-binding`, and the root's own `version`, `date`
and `logs`. Plus the `float`/`integer` width loss above.

**Not dropped:** the root's `fix-version`, `sendercompid` and `targetcompid`
become the branch's `FixBranchInfo`, and `type` is read for the
`BuySide`/`SellSide` role that says which way the session pair points.

## Errors

`Error::Parse { target: "cfb", position, reason }` for malformed input, so a
failure names the tag and the byte position; `Error::InvalidRecord` where an
existing FIX refusal already has a shape. Typed errors are required for: a
`type` outside the eight; a `domain` outside the two; a `tag-constraint`
whose `name` misses the vocabulary; a nested `grammar` with no counter; a
nesting depth past the guard; and a document that is not well-formed.
`encoding="US-ASCII"` is a UTF-8 subset and is read as UTF-8; document that.

## Delivery

**Dependency.** A streaming pull parser, not a DOM crate — the largest file
is 6.3 MB and 300k elements, and only two of fourteen top-level children are
read, so the parser must skip by depth without materializing a tree.
`quick-xml` with `default-features = false` is the fit; pin the exact current
release the way the other exact-pinned crates are, and justify it in a
manifest comment the way `flate2` and `memchr` are. No `encoding` feature:
the corpus is ASCII and quick-xml reads UTF-8. Unescaping goes through the
crate's own `unescape`, never a hand-rolled entity table.

**Tests** go in the FIX module's tests, fixtures as inline `&str` in the
exact shape of the extract below, handles the way the store cases already
build them. Cover: a small vocabulary; a flat binding; a two-level nested
group; the worked tree below asserted node for node; the vocabulary half
inserting into a registry read from the committed dictionary, both outcomes
pinned — same tag and same name replaces, same tag and a different name is a
typed conflict; a group's counter consumed rather than emitted; a missing
`alt`; an expression-valued `required`; a constraint with no validity; a
`<description />` contributing no key; an empty `grammar`; a nested `grammar`
with no counter; a duplicate tag at one level; depth 5; the `utc-date` alias;
and a malformed file whose error names the tag and the byte position.

Then the branch record, which is what the root element is for: the root's
`fix-version`, `sendercompid` and `targetcompid` landing on the named
branch's `FixBranchInfo`; a file parsed with no branch landing in the
standard branch and declaring no session; and a `BuySide` file and a
`SellSide` file declaring one session in opposite orders, both matched.

**Benchmark** a full parse: a new CBlock group in the FIX benchmark target.
Add an allocation case to the counting-allocator target for the parse's
steady-state cost.

**Docs** extend the FIX page; no new page, no nav change. Contract first,
then one runnable asserted example, then the losses table, then the measured
numbers. The `utc-date` alias also touches the datatype page's logical-name
table.

**Verify** what `AGENTS.md` requires: `cargo fmt`, warning-free Clippy,
workspace tests with default features and `parquet iceberg`, Rust 1.85
default and `--no-default-features --lib`, rustdoc with warnings denied, the
fix and datatype benches, the docs-example checker, and the strict mkdocs
build. Rust-only: no Python or Node work.

## The shape, verbatim

Trimmed but literal extract of a production file (`Bloomberg_FIX44_DropCopy`,
a `BuySideFIXCPluginCBlock`). Element order, attribute order, entity escaping
and self-closing forms are as observed; build fixtures in this exact shape.

```xml
<?xml version="1.0" encoding="US-ASCII"?>
<cplugin-configuration type="com.ullink.ulbridge2.toolkit.plugins.fix.model.state.cblock.BuySideFIXCPluginCBlock" version="1.2" logs="false" date="Fri 2019-09-13 13:07:00" fix-version="4.4" targetcompid="..." sendercompid="...">
	<history>
		<version date="2006-09-06 11:46:49" owner="ullink">First version released by ULLINK</version>
		<version date="2006-09-22 16:41:44" owner="admin">3 main changes in order to support custom values for TAG 63 coming from Bloomberg:
* change mapping SettlType and add 40=&gt;spot, 54=&gt;forward
* change type of tag 63 from char to String</version>
		<version date="2018-10-22 11:41:25" owner="fcolombat" />
		<version date="2019-02-15 13:07:06" owner="sgrbic">Correct Tag 423 Yield 9 again to 1 when 669 present</version>
	</history>
	<cvs-revision>$Revision: 1.3 $ $Date: 2025.09.19 16:21:37 $</cvs-revision>
	<description>Standard Buy Side 4.4</description>
	<message-types>
		<message-type value="6 Inbound" description="Indication of Interest" rejection="Message type 6 Inbound (Indication of Interest) is not supported by this adapter" supported="false" />
		<message-type value="8" description="Execution Report" rejection="Message type 8 (Execution Report) is not supported by this adapter" supported="true" />
		<message-type value="P Report Ack" description="Allocation Report ACK" rejection="Message type P Report Ack (Allocation ACK) is not supported by this adapter" supported="false" />
		<message-type value="c SDR" description="Security Definition Request" rejection="Message type c SDR (Security Definition Request) is not supported by this adapter" supported="false" />
		<message-type value="d SL" description="Security Definition" rejection="Message type d SL (Security Definition) is not supported by this adapter" supported="false" />
	</message-types>
	<inbound-message-type-mappings />
	<outbound-message-type-mappings>
		<entry key="allocation" value="J" />
		<entry key="allocationreportack" value="P Report Ack" />
		<entry key="replacerequestmultileg" value="AC" />
	</outbound-message-type-mappings>
	<vocabulary>
		<vocabulary-tag name="1" alt="Account" type="string" read-only="false">
			<description>Account mnemonic as agreed between buy and sell sides, e.g. broker and institution or investor/intermediary and fund manager.</description>
		</vocabulary-tag>
		<vocabulary-tag name="4" alt="AdvSide" type="char" read-only="false">
			<description>Broker's side of advertised trade Valid values: B = Buy S = Sell X = Cross T = Trade</description>
		</vocabulary-tag>
		<vocabulary-tag name="6" alt="AvgPx" type="float" read-only="false">
			<description>Calculated average price of all fills on this order. For Fixed Income trades AvgPx is always expressed as percent-of-par, regardless of the PriceType (423) of LastPx (3).</description>
		</vocabulary-tag>
		<vocabulary-tag name="10" alt="CheckSum" type="string" read-only="false">
			<description>Three byte, simple checksum (see Volume 2: ???Checksum Calculation??? for description). ALWAYS LAST FIELD IN MESSAGE; i.e. serves, with the trailing &lt;SOH&gt;, as the end-of-message delimiter.</description>
		</vocabulary-tag>
		<vocabulary-tag name="10001" alt="ExludedDealers" type="boolean" read-only="false">
			<description />
		</vocabulary-tag>
		<vocabulary-tag name="10015" alt="DealerParQuote" type="float" />
		<vocabulary-tag name="22830" alt="NoteLegRefID" type="string" />
	</vocabulary>
	<grammar-binding type="7">
		<grammar checkordering="false">
			<tag-constraint name="8" activated="false" read-only="true" part="header" required="true">
				<string-validity regexp=".*" domain="all-values" />
			</tag-constraint>
			<tag-constraint name="9" activated="false" read-only="true" part="header" required="true">
				<integer-validity domain="all-values">
					<integer-range min="minimum" max="maximum" />
				</integer-validity>
			</tag-constraint>
			<tag-constraint name="35" activated="true" read-only="true" part="header" required="true">
				<string-validity regexp="^7$" domain="ranges" />
			</tag-constraint>
			<tag-constraint name="115" activated="true" read-only="false" part="header" required="false">
				<string-validity regexp=".*" domain="all-values" />
			</tag-constraint>
			<grammar checkordering="false">
				<tag-constraint name="555" activated="true" read-only="false" part="body" required="false">
					<integer-validity domain="ranges">
						<integer-range min="1" max="maximum" />
					</integer-validity>
				</tag-constraint>
				<tag-constraint name="556" activated="true" read-only="false" part="body" required="false">
					<string-validity regexp=".*" domain="all-values" />
				</tag-constraint>
				<tag-constraint name="739" activated="true" read-only="false" part="body" required="false">
					<utc-date-validity />
				</tag-constraint>
				<tag-constraint name="740" activated="true" read-only="false" part="body" required="false">
					<string-validity regexp=".*" domain="all-values" />
				</tag-constraint>
				<tag-constraint name="955" activated="true" read-only="false" part="body" required="false">
					<string-validity regexp="^[0-9]{4}((0[1-9])|10|11|12)$" domain="ranges" />
				</tag-constraint>
				<tag-constraint name="956" activated="true" read-only="false" part="body" required="false">
					<utc-date-validity />
				</tag-constraint>
			</grammar>
			<grammar checkordering="false">
				<tag-constraint name="711" activated="true" read-only="false" part="body" required="false">
					<integer-validity domain="ranges">
						<integer-range min="1" max="maximum" />
					</integer-validity>
				</tag-constraint>
				<tag-constraint name="311" activated="true" read-only="false" part="body" required="true">
					<string-validity regexp=".*" domain="all-values" />
				</tag-constraint>
				<tag-constraint name="312" activated="true" read-only="false" part="body" required="false">
					<string-validity regexp=".*" domain="all-values" />
				</tag-constraint>
				<tag-constraint name="309" activated="true" read-only="false" part="body" required="false">
					<string-validity regexp=".*" domain="all-values" />
				</tag-constraint>
				<tag-constraint name="305" activated="true" read-only="false" part="body" required="false">
					<string-validity regexp="^[1-9A-J]$" domain="ranges" />
				</tag-constraint>
				<grammar checkordering="false">
					<tag-constraint name="457" activated="true" read-only="false" part="body" required="false">
						<integer-validity domain="ranges">
							<integer-range min="1" max="maximum" />
						</integer-validity>
					</tag-constraint>
					<tag-constraint name="458" activated="true" read-only="false" part="body" required="true">
						<string-validity regexp=".*" domain="all-values" />
					</tag-constraint>
					<tag-constraint name="459" activated="true" read-only="false" part="body" required="true">
						<string-validity regexp="^[1-9A-J]$" domain="ranges" />
					</tag-constraint>
				</grammar>
				<tag-constraint name="462" activated="true" read-only="false" part="body" required="false">
					<integer-validity domain="ranges">
						<integer-range min="1" max="13" />
					</integer-validity>
				</tag-constraint>
				<tag-constraint name="463" activated="true" read-only="false" part="body" required="false">
					<string-validity regexp=".*" domain="all-values" />
				</tag-constraint>
				<tag-constraint name="310" activated="true" read-only="false" part="body" required="false">
					<string-validity regexp="^(CS|FUT|OPT|NONE)$" domain="ranges" />
				</tag-constraint>
			</grammar>
		</grammar>
	</grammar-binding>
	<normalization-binding>… skipped whole; see the rules …</normalization-binding>
	<reject-binding />
	<flow-filter-binding>
		<flow-filters flowType="inbound">
			<flow-filter filter-message="Filtering Trade Cancel messages">
				<condition>
					<expression value="$35=&quot;8&quot; and $150=toChar(&quot;H&quot;)" />
				</condition>
			</flow-filter>
		</flow-filters>
		<flow-filters flowType="outbound" />
	</flow-filter-binding>
	<maps>… skipped whole …</maps>
</cplugin-configuration>
```

Read from that extract, and encode in the fixtures, that: `history/version`
is sometimes self-closing and sometimes carries multi-line text with
entities, so the skip must assume neither; `message-type/@value` composite
keys survive verbatim into `outbound-message-type-mappings/entry/@value`;
`vocabulary-tag` appears in three shapes - full with text description, full
with `<description />`, and bare self-closing with no `read-only` at all; a
`grammar` mixes `header` and `body` constraints in one flat list with a
nested `grammar` inline between two body constraints, and that nested group's
first `tag-constraint` (`457`) is the counter with no `rg-name` anywhere;
`integer-validity domain="all-values"` still nests an `integer-range` with
both sentinels; and every expression-bearing attribute is entity-escaped, so
the parser must unescape before it can decide a value is not a boolean.

## The tree, worked

The `grammar-binding type="7"` above, after both passes, is exactly this
shape. `!` marks non-null, `?` nullable; the trailing comment is the metadata
carried on that field. Assert it node for node.

```
"7"                                        Struct !            (name is the MsgType, verbatim)
├── BeginString                            utf8 !               fix:tag=8   fix:description=…
├── BodyLength                             int32 !              fix:tag=9
├── MsgType                                utf8 !               fix:tag=35
├── OnBehalfOfCompID                       utf8 ?               fix:tag=115
├── NoLegs                                 List<item:Struct !> ?  fix:tag=555   (counter, consumed)
│   └── item                               Struct !
│       ├── LegCurrency                    utf8 ?               fix:tag=556
│       ├── LegIssueDate                   date32 ?             fix:tag=739
│       ├── LegRepoCollateralSecurityType  utf8 ?               fix:tag=740
│       ├── LegContractSettlMonth          utf8 ?               fix:tag=955
│       └── LegInterestAccrualDate         date32 ?             fix:tag=956
└── NoUnderlyings                          List<item:Struct !> ?  fix:tag=711   (counter, consumed)
    └── item                               Struct !
        ├── UnderlyingSymbol               utf8 !               fix:tag=311
        ├── UnderlyingSymbolSfx            utf8 ?               fix:tag=312
        ├── UnderlyingSecurityID           utf8 ?               fix:tag=309
        ├── UnderlyingSecurityIDSource     utf8 ?               fix:tag=305
        ├── NoUnderlyingSecurityAltID      List<item:Struct !> ?  fix:tag=457   (counter, consumed)
        │   └── item                       Struct !
        │       ├── UnderlyingSecurityAltID        utf8 !       fix:tag=458
        │       └── UnderlyingSecurityAltIDSource  utf8 !       fix:tag=459
        ├── UnderlyingProduct              int32 ?              fix:tag=462
        ├── UnderlyingCFICode              utf8 ?               fix:tag=463
        └── UnderlyingSecurityType         utf8 ?               fix:tag=310
```

(The two `utc-date` leaves are `date32` because that is what `utcdateonly`
resolves to today and what the new `utcdate` alias must answer; the tree
above is written in the core's own datatype spelling, which is what
`DataType::Display` prints.)

Five things that tree must make obvious in review:

1. `NoLegs` and `NoUnderlyings` are siblings produced by two consecutive
   `grammar` elements at the same depth, so the builder recurses on `grammar`
   and never assumes one group per binding.
2. Each counter - `555`, `711`, `457` - exists only as its group field's
   `fix:tag` and never as a leaf.
3. A group's nullability comes from its counter's `required`, while the
   `item` Struct is always non-null.
4. Header tags `8`, `9`, `35`, `115` are flat siblings of the groups because
   `part` is dropped.
5. Every leaf's name, `DataType` and `fix:description` come from the
   `vocabulary` pass; the grammar contributes nothing but nullability,
   nesting and position.

The `UnderlyingInstrument` component is not represented anywhere - its fields
are simply the `item` Struct's children, and the fixture must not try to name
it.
