# Ullink CBlock (`.cfb`) → FIX definitions

Parse an Ullink `.cfb` CBlock configuration read through an existing
`IOBase` handle and answer the FIX field definitions it declares. Follow
`AGENTS.md`.

## What is already landed

The FIX phase this builds on is merged; there is no prerequisite brief left
to read (both superseded root briefs were deleted in `5051629`). Read the
code instead, and reuse it unchanged:

| what | where |
| --- | --- |
| `FixBranch`, `FixId`, `FixKey`, module docs | the FIX module |
| `FixField` / `FixFieldMut` accessors over `fix:` | the FIX field views |
| `FixRegistry`, `insert`, `update`, tiered resolution | the FIX registry |
| `from_handle` / `write_into` over one folder handle | the registry's store |
| the process default | the FIX module |
| `FixMsg` | the FIX module |
| module tests (one file, inline fixtures) | the FIX module's tests |
| the seed dictionary | the committed dictionary |
| the page to extend | the FIX documentation page |
| the bench target | the FIX benchmark target |

`FixField` carries `fix:branch`, `fix:tag`, `fix:tags`, `fix:aliases`,
`fix:description`; nesting needs no second type (a component is a Struct, a
repeating group a List of that Struct whose counter tag is the group field's
own `fix:tag`); a `List` item field is named `item`.

## Scope

Read `vocabulary` and `grammar-binding`. Nothing else.

Add nothing beside the one constructor: no `cfb:` protocol, no `Scheme`
variant, no `MediaType`, no new reserved metadata key, no document or schema
type beside `Field`. Whatever a `.cfb` declares that the core cannot already
express is dropped in this phase, and the module docs name every dropped
thing.

Skip the entire `normalization-binding` layer. That is message transcoding,
not schema, and belongs to a later phase - do not sketch an evaluator for its
expression language now.

## Entry point

Two facts of the current code decide the signature, and both differ from the
first draft of this brief:

1. `FixField<'field>` is a macro-generated newtype over `ProtocolField<'field>`
   that wraps a `&'field Field`. It
   cannot own a parse result, and an associated constructor that ignores its
   own lifetime does not belong on it.
2. `FixRegistry::insert` refuses a field carrying no `fix:tag` - "a field
   enters only with a `fix:tag`".
   A message root is a Struct named by a MsgType and has no tag, so **message
   roots cannot enter a `FixRegistry`**. There is no existing "key for message
   roots" to reuse; do not invent a synthetic tag, a `FixBranch` cut from
   `fix-version`, or a second keying scheme to smuggle one in.

So the constructor keeps the name and moves to the type that already owns a
handle constructor and owns fields:

```rust
impl FixRegistry {
    /// Parses an Ullink CBlock configuration into the vocabulary it declares
    /// and the message roots its grammar bindings describe.
    ///
    /// `branch` names the dialect the file describes, because a `.cfb` never
    /// names itself; `None` reads it into the standard branch. The root
    /// element's `fix-version`, `sendercompid` and `targetcompid` become
    /// that branch's `FixBranchInfo`.
    pub fn from_cfb(
        handle: &dyn IOBase,
        branch: Option<&FixBranch>,
    ) -> Result<(Self, Vec<Field>)>;
}
```

New module a new CBlock module inside the FIX module, declared `mod cfb;` in
the FIX module beside `store`; nothing new is re-exported from
the crate root because nothing new is public but the method.

The registry half holds the `vocabulary` fields, each with its `fix:tag`, and
is directly insertable, writable through `write_into`, and mergeable into a
registry seeded from the committed dictionary. The `Vec<Field>` half holds the message
roots in binding document order. Merging the two halves is the caller's
business, and the constructor takes no seed registry: `from_cfb` answers what
one file says.

Read the file with `read_all_bytes()`, the way the store
reads a shard.

## The format

Every fact below is taken from 81 production files (133 MB, largest 6.3 MB
and 300k elements; a representative one is 1'522'455 bytes over 31'587
lines). All are well-formed `<?xml version="1.0" encoding="US-ASCII"?>`,
tab-indented, Unix LF. You will not have those files: build fixtures by hand
from this description and the verbatim extract below.

Root: `cplugin-configuration[type version fix-version date? logs?
targetcompid? sendercompid?]`.

**Three of those attributes are the branch record, and reading them is the
first thing this parser does.** A `.cfb` is one counterparty's dictionary,
and it states at the top exactly what a dialect declares about itself:
`fix-version` (e.g. `4.4`) is the FIX version it speaks, and `sendercompid`
/ `targetcompid` name its session. That is the `FixBranchInfo` the FIX
versioning brief defines - `prompts/fix-versioning/`, rule `P2-R9b` - so read them into one and stop
dropping them:

- `fix-version` parses through the FIX layer's version mapping into a
  `Version`. It never becomes a *branch*: a branch must start with an ASCII
  letter, so `4.4` is not one, and the confusion between "which dialect" and
  "which version of the protocol" is exactly what bundling them on the
  branch record ends.
- `sendercompid` / `targetcompid` are stored as they arrive and compared
  folded. **The `type` attribute is what says which way the pair points:**
  its `BuySide`/`SellSide` portion gives the plugin's role, so the declared
  pair is the session seen from *this* side, and the other side sees it
  reversed. That is why a reader matching a session tries both orders.
- `version` (the CBlock format's own, e.g. `1.2`), `date` and `logs` stay
  dropped. They describe the file, not the dialect.

**The branch name does not come from the file**, because a `.cfb` never
names itself. The caller supplies it, and that is a parameter of the
constructor below; absent, the parse lands in the standard branch and
declares no session, which is the right answer for a file that is only
being read for its vocabulary.

Children, in document order: `history`, `cvs-revision`, `description`,
`message-types`, `inbound-message-type-mappings`,
`outbound-message-type-mappings`, `vocabulary`, `grammar-binding`,
`normalization-binding`, `reject-binding?`, `flow-filter-binding?`, `maps`,
`options?`, `noe-normalization-binding*`.

Skipped siblings are routinely self-closing (`<inbound-message-type-mappings />`,
`<reject-binding />`, `<flow-filters flowType="outbound" />`) and routinely
deep and text-bearing (`history/version` carries multi-line text with
entities). A skip must tolerate both, so skip by depth counting, never by
assuming an empty element or a known child set.

### `vocabulary`

`vocabulary-tag[name alt? type read-only? ref?]` with an optional
`description` child.

- `name` is the decimal FIX tag.
- `alt` is the FIX field name, absent in a handful of entries - fall back to
  the tag rendered as text.
- `ref` names the data tag this one is the length of: a FIX Len/data pair
  whose length tag carries no independent value. Dropped.
- `read-only` is often absent on custom high-numbered tags.
- The `description` child is sometimes absent, and sometimes present but
  empty (`<description />`). Its text carries XML entities (`&lt;SOH&gt;`,
  `&quot;`) that must be unescaped - never treat descriptions as opaque
  bytes.
- `type` is exactly one of `string`, `char`, `integer`, `float`, `boolean`,
  `utc-date`, `utc-timestamp`, `utc-time-only`. Anything else is a typed
  error.

### `message-types`

`message-type[value description rejection supported]`. `value` is a FIX
MsgType optionally followed by a space and a qualifier - observed qualifiers
are `Inbound`, `Outbound`, `Report`, `Report Ack`, `SD`, `SL`, `SDR`, `SLR`,
`4.2`. Treat the whole string as an opaque key; never split on the space to
recover a direction. The same composite keys reappear verbatim as `entry`
values in `outbound-message-type-mappings` (`value="P Report Ack"`), which
confirms they are keys and not a parseable pair. `supported="false"` is the
common case (2231 of 2988). The element is skipped in this phase; the rule
matters because `grammar-binding/@type` carries the same composite strings.

### `grammar-binding`

`grammar-binding[type merge-mode?]` holds exactly one `grammar`. Its `type`
is a MsgType such as `7`, usually equal to a `message-type` value but not
necessarily, and bindings exist for unsupported types - require neither
match.

`grammar[checkordering? activated? rg-name? merge-mode?]` holds interleaved
`tag-constraint*` and nested `grammar*`, and nests to observed depth 5.

`grammar` is the only container element and the only recursion point in the
format. Nested `grammar`s are siblings as often as they are children: one
`grammar` routinely holds several, separated by plain `tag-constraint`s - a
leg group closing on `</grammar>` and an underlying group opening on the next
line, that underlying group itself holding a security-alt-id group. So the
parser is one recursive function over `grammar`, never a two-level special
case.

A nested `grammar` is a FIX repeating group whose first child is the counter
tag's `tag-constraint`. `rg-name`, when present, names that same tag, but it
is absent from most nested groups: the first-`tag-constraint` path is the hot
path, not the fallback. Resolution order is `rg-name`, else the first
`tag-constraint`, else skip the group with a typed error.

There is no component element and no reference mechanism. FIX component
blocks - `Instrument`, `UnderlyingInstrument`, `LegInstrument` - are
flattened into plain sibling `tag-constraint`s at the point of use, repeated
in full in every `grammar` that needs them. Do not try to recover components,
factor repeated runs of tags into shared sub-Structs, or dedupe two grammars
carrying the same tag sequence. Only repeating groups are structure, and
every repeating group is a `grammar`.

Three real structural exceptions exist in the corpus and must be handled
rather than asserted away: an empty `grammar`; a `grammar` whose first child
is another `grammar`; and one file where `rg-name` disagrees with the first
`tag-constraint`.

### `tag-constraint`

`tag-constraint[name part required? activated? read-only? merge-mode?]`.

- `name` is the decimal tag, and always resolves in the same file's
  `vocabulary` - no dangling reference exists in any of the 81 files, so a
  miss is a genuine typed error.
- `part` is `header`, `body` or `trailer`.
- Everything but `name` and `part` is optional. Define and document each
  default rather than assuming `false`.
- `required` is usually `true` or `false`, but is sometimes a condition
  expression such as `$59 = '6' and empty($126)` or `$167 = "FXSPOT" or $167
  = "FXFWD"`. Parse it as boolean-or-expression, treat any expression as
  not-required for schema purposes, and never fail on it. The attribute is
  entity-escaped, so it must be unescaped before it can even be tested
  against `true`/`false`.

A `tag-constraint` has zero or one validity child - absent in 81382 of 301372
constraints, meaning unconstrained - plus an optional `condition` holding
`expression[value]` that makes the constraint conditional. One file pairs two
validity elements, so tolerate more than one.

Validity elements: `string-validity[regexp domain]`,
`char-validity[regexp domain]`, `integer-validity[domain]` over one or more
`integer-range[min max]`, `float-validity[domain]` over `float-range[min max]`,
`boolean-validity[value domain]` whose `value` is the Java literal
`BooleanValidity.UNDEFINED` or `BooleanValidity.TRUE`, and the always-empty
`utc-date-validity`, `utc-timestamp-validity`, `utc-time-only-validity`.
`domain` is `all-values` or `ranges`; anything else is a typed error. `min`
and `max` are a decimal literal or the sentinel `minimum`/`maximum` meaning
unbounded on that side, and both sentinels occur in one element
(`min="minimum" max="maximum"` under `domain="all-values"`), so an
`all-values` integer validity still carries a range child and must not be
special-cased into an empty one.

All of it is dropped in this phase. It is specified here so the parser can
consume it without guessing, and so the docs can name the loss exactly.

### `merge-mode`

Valued `assert`, `update`, or `update|ignore`; appears on `vocabulary-tag`,
`tag-constraint`, `grammar`, `grammar-binding`, `normalization-binding` and
`map` in two overlay files that patch a base configuration. Overlay merging
is out of scope: parse the attribute, ignore it, and document that a `.cfb`
carrying it is read standalone.

## Type resolution

Resolve `vocabulary-tag/@type` through `DataType::from_str`. Logical names
fold to lowercase with `_`, `-` and space removed
(the datatype grammar's own name fold), so seven of the eight already resolve:

| `.cfb` type | resolves through | to |
| --- | --- | --- |
| `string` | grammar (the grammar's text spellings) | `utf8` |
| `char` | grammar (the grammar's text spellings) | `utf8` |
| `integer` | grammar (the grammar's integer spellings) | `int32` |
| `float` | grammar (the grammar's float spellings) | `float32` |
| `boolean` | grammar (the grammar's boolean spellings) | `bool` |
| `utc-timestamp` | `LOGICAL_NAMES["utctimestamp"]` | `timestamp(ns,"UTC")` |
| `utc-time-only` | `LOGICAL_NAMES["utctimeonly"]` | `time64(ns)` |
| `utc-date` | **nothing** | - |

Three decisions, each explicit:

1. **`utc-date` does not resolve**, because it folds to `utcdate` and the
   registry spells that name `utcdateonly`
   (the logical-name table spells it `utcdateonly`). Add the alias generically rather
   than mapping it privately in the parser. That is three edits pinned by one
   verbatim-comparison test:
   - the logical-name table - `("utcdate", DataType::Date32)` beside
     `utcdateonly`;
   - its mirror in the datatype tests - the same entry in `registered()`,
     which `the_registry_is_the_documented_mapping_and_holds_no_repeat`
     compares against `LOGICAL_NAMES` element for element;
   - the datatype page - the `UTCDateOnly` row gains the second spelling,
     the way `Exchange`, `mic` already share one row.

   The name is not one the Arrow/SQL grammar owns, so this respects the
   `AGENTS.md` rule on the logical-name registry.

2. **`float` stays `float32` and `integer` stays `int32`.** They are the
   grammar's generic answers and the wrong shape for FIX money and sequence
   numbers, where `LOGICAL_NAMES` already spells `price` and `qty` as
   `decimal64(18,8)` and `seqnum` as `int64` - but a `.cfb` says nothing
   about which tag is money, and the second pass never invents a type. Do not
   promote by tag, do not consult the seeded registry mid-parse. Document
   this as the phase's principal known loss, and document the consequence:
   merging a `.cfb` vocabulary into a registry seeded from the committed dictionary
   *replaces* tag 6 `AvgPx` `decimal64(18,8)` with `float32`, because
   `insert` replaces wholesale on an identity match.

3. **Nothing else is inferred from the validity children.** A `regexp` that
   pins a length does not become `ascii(n)`; a `domain="ranges"` does not
   become an `AsciiEnum`.

## The mapping

Two passes over one file: `vocabulary` first to build the dictionary,
`grammar-binding` second to build the message trees. The second pass never
invents a type.

### Vocabulary pass

A `vocabulary-tag` becomes a `Field`:

- named by `alt`, else by the decimal tag rendered as text;
- carrying the `DataType` its `type` resolves to;
- `fix:tag` from `name`;
- `fix:description` from the `description` child **only when that child is
  present and its unescaped text is non-empty** - `<description />`
  contributes no key rather than an empty one;
- nullable, because a `vocabulary-tag` says nothing about presence.

Dictionary entries are immutable. A grammar never edits one: it clones it and
overrides the clone's nullability, so a tag bound `required="true"` in one
message and `required="false"` in another yields two independent `Field`s off
one dictionary entry.

### Grammar pass

A `grammar-binding` becomes exactly one root `Field`: a non-null Struct named
by its `type` verbatim, spaces included, so `7` and `P Report Ack` are both
legal field names. The MsgType is carried by the name alone - no reserved
metadata key is introduced to hold it, and the composite qualifier is never
split.

The root's children are the `grammar`'s children in document order, flattened
across `part`: a `tag-constraint` becomes a leaf, a nested `grammar` becomes
a repeating-group field, and the two interleave exactly as the file
interleaves them. Do not regroup by `part` into header/body/trailer
sub-Structs and do not sort by tag. Document order is the schema order and
`part` is dropped, which means a `header` constraint and a `body` constraint
sit as siblings in one flat Struct.

**A leaf `tag-constraint`** resolves its `name` in the same file's
`vocabulary`, clones that dictionary `Field`, and sets `nullable =
!required`, an expression-valued `required` counting as nullable. The leaf
keeps the dictionary's name, `DataType`, `fix:tag` and `fix:description`
unchanged; the constraint's own validity child contributes nothing.

**A nested `grammar`** becomes one repeating-group `Field` whose `DataType`
is a List of non-null Struct. Its first `tag-constraint` is the group counter
and is consumed, not emitted: the group field takes that counter's name,
`fix:tag` and `fix:description` from the counter's dictionary entry, and
`nullable = !counter.required`, while the counter's own integer `DataType` is
discarded because the list length already carries it. The List's item field
is named `item`, is non-null, and is a Struct whose children are the nested
`grammar`'s remaining children - every `tag-constraint` after the first and
every `grammar` deeper still - in document order, built by the same rules
recursively. Observed nesting reaches depth 5, so recurse rather than unroll,
and raise a typed error past a documented guard depth rather than overflow
the stack.

**A duplicate tag at one Struct level** is possible now that `part` is
dropped and a tag can be bound in both `header` and `body`. It is not an
error and not a deduplication: keep both children in document order and
disambiguate the second and later names deterministically with a documented
suffix, leaving `fix:tag` identical on all of them so the tag stays
recoverable.

**The structural exceptions** land here: a root `grammar` with no children
becomes an empty non-null Struct rather than a failure; a nested `grammar`
that is empty, or whose first child is another `grammar`, has no counter to
name it, so it is skipped with a typed error naming the byte position while
the parent keeps its remaining children.

### Dropped, by name

The module docs list these as the phase's known losses: `part`, `activated`,
`read-only`, `ref`, `checkordering`, `rg-name` beyond group naming,
`condition` and every `expression`, `regexp`, `domain`, every range and
sentinel, every validity element, `merge-mode`, `message-types` and both
mapping tables, `history`, `cvs-revision`, `description` on the root,
`normalization-binding`, `reject-binding`, `flow-filter-binding`, `maps`,
`options`, `noe-normalization-binding`, and the root's own `version`, `date`
and `logs`. Plus the `float`/`integer` width loss above.

**Not dropped any more:** the root's `fix-version`, `sendercompid` and
`targetcompid` become the branch's `FixBranchInfo`, and `type` is read for
the `BuySide`/`SellSide` role that says which way the session pair points.
Only the role is kept from it - the Java class name itself is not stored.

## Errors

Use `Error::Parse { target: "cfb", position, reason }` for malformed input,
so a failure names the tag and the byte position, and `Error::InvalidRecord`
where an existing FIX refusal already has a shape. Typed errors are required
for: a `type` outside the eight; a `domain` outside the two; a
`tag-constraint` whose `name` misses the vocabulary; a nested `grammar` with
no counter; a nesting depth past the guard; and a document that is not
well-formed. `encoding="US-ASCII"` is a UTF-8 subset and is read as UTF-8;
document that.

## Delivery

**Dependency.** Add a streaming pull parser, not a DOM crate - the largest
file is 6.3 MB and 300k elements, and only two of fourteen top-level children
are read, so the parser must skip by depth without materializing a tree.
`quick-xml` with `default-features = false` is the fit; pin the exact current
release the way `smol_str`, `saphyr-parser`, `toml` and `twox-hash` are
pinned, and justify it in a the crate manifest comment the way `flate2`,
`memchr` and `snap` are. No `encoding` feature: the corpus is ASCII and
quick-xml reads UTF-8. Unescaping goes through the crate's own `unescape`,
never a hand-rolled entity table.

**Tests** go in the FIX module's tests, fixtures as inline `&str` in the
exact shape of the extract below, handles via `Folder::temporary()` the way
the store cases already do. Cover: a small vocabulary; a flat binding; a
two-level nested group; the worked tree below asserted node for node; the
vocabulary half inserting into a registry read from the committed dictionary, both
outcomes pinned - same tag and same name replaces, same tag and a different
name is a typed conflict; a group's counter consumed rather than emitted; a
missing `alt`; an expression-valued `required`; a constraint with no
validity; a `<description />` contributing no key; an empty `grammar`; a
nested `grammar` with no counter; a duplicate tag at one level; depth 5; the
`utc-date` alias; and a malformed file whose error names the tag and the byte
position.

Then the branch record, which is what the root element is for: the root's
`fix-version`, `sendercompid` and `targetcompid` landing on the named
branch's `FixBranchInfo`; a file parsed with no branch landing in the
standard branch and declaring no session; and a `BuySide` file and a
`SellSide` file declaring one session in opposite orders, both matched.

**Benchmark** a full parse: a new CBlock group in the FIX benchmark target beside `resolve`, `mutate` and
`store`. Add an allocation case to the counting-allocator test target for the parse's
steady-state cost - that file owns the process's one counting allocator.

**Docs** extend the FIX documentation page; no new page, no nav change. Put the contract
first, then one runnable asserted example, then the losses table, then the
measured numbers. The `datatype` alias also touches the datatype page's
logical-name table.

**Verify** what `AGENTS.md` requires for this surface: `cargo fmt`,
warning-free Clippy, workspace tests with default features and
`parquet iceberg`, Rust 1.85 default and `--no-default-features --lib`,
rustdoc with warnings denied, the fix and datatype benches,
the repository's docs-example checker, and `python -m mkdocs build --strict`.
Rust-only phase: no Python or Node work.

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
	<normalization-binding>
		<normalization>
			<tag-normalization tag-name="TARGETLOCATIONID" read-only="false" part="header">
				<mapping-condition />
				<mapping-expression>
					<expression value="$143" />
				</mapping-expression>
			</tag-normalization>
			<tag-normalization tag-name="ADVREFID" read-only="false" part="body">
				<mapping-condition>
					<expression value="$5 = &quot;C&quot; or $5 = &quot;R&quot;" />
				</mapping-condition>
				<mapping-expression>
					<expression value="$3" />
				</mapping-expression>
			</tag-normalization>
			<tag-normalization tag-name="ADVTRANSTYPE" read-only="false" part="body">
				<mapping-condition />
				<mapping-expression>
					<expression value="lookup(&quot;AdvTransType&quot;, $5)" />
				</mapping-expression>
			</tag-normalization>
			<condition-expression />
		</normalization>
	</normalization-binding>
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
	<maps>
		<map name="ACCOUNTTYPE" read-only="false">
			<description>Used for the decoding of UlMessage tag ACCOUNTTYPE</description>
			<entries>
				<entry key="floortrader" value="4" />
				<entry key="housetrader" value="3" />
			</entries>
		</map>
	</maps>
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
