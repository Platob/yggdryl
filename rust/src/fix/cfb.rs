//! An Ullink CBlock configuration, read for the FIX definitions it declares.
//!
//! A `.cfb` is one counterparty's dictionary. It states at the top the FIX
//! version a dialect speaks, and then two things worth reading: a
//! `vocabulary` of tags, and a `grammar-binding` per message type describing
//! that message's tree. Everything else in the file describes the file, the
//! transcoding, or the plugin, and is skipped - the version among it, because
//! which version a run reads at is the codec's pin and not a vocabulary's.
//!
//! # One entry point, one parse
//!
//! [`FixRegistry::from_cfb_file`] answers the whole file: a dictionary of its
//! scalar fields, the [code sets](super::codes) its maps decode, its
//! components, groups and message definitions, plus the message roots. It
//! takes the dialect name from the file's own stem when the caller supplies
//! none, stamps every field it produces as a member of the dialect in its
//! `FIX:sources`, and holds the dialect's entry - its id, the file's name
//! and the role of the plugin the root's `type` names - in the dictionary's
//! sources catalog.
//!
//! A vocabulary is a dictionary rather than a list of fields, because a code
//! set is named and owned by the dictionary: a field carries the *name* of
//! the set its maps decode, so a bare `Vec<Field>` would hand a reader
//! fields naming vocabularies nothing states. So folding one counterparty's
//! file into a dictionary that exists is
//! [`FixRegistry::add_cfb_file`] - one parse, one fold, the sets travelling
//! with the fields that read by them.
//!
//! # Two passes, and the second never invents a type
//!
//! `vocabulary` builds the dictionary; `grammar-binding` builds the message
//! roots out of it. A constraint borrows its resolved field and records its
//! own nullability. A nested grammar resolves its opening counter to int32 in
//! the vocabulary and becomes a separately named Serie of components that
//! states the counter as its `FIX:counter`: the count is the serie's length,
//! so no message lists the counter beside it.
//!
//! # Only repeating groups are structure
//!
//! There is no component element and no reference mechanism. FIX component
//! blocks - `Instrument`, `UnderlyingInstrument`, `LegInstrument` - are
//! flattened into plain sibling constraints at the point of use and repeated
//! in full wherever they are needed. Nothing here recovers them or factors
//! repeated runs into shared sub-structs. Every repeating group is a nested
//! `grammar`, and one recursive function reads all of them - nested grammars
//! are siblings as often as children. Its entries become named component
//! definitions and its collection becomes a group definition; both carry no
//! synthetic FIX tag.
//!
//! Two grammars declaring one component or one group structurally alike
//! declare one definition, whatever they name it. The structure is the
//! identity - the members in order, each its tag and the field it reads, a
//! nested group its counter and the structure it repeats - and nothing else
//! is: not how strictly a member was stated, not the `rg-name` the grammar
//! spelled, not a description, and not which dialect declared it. So the
//! first declaration's name and spelling hold, a member one grammar states
//! as `required` and another does not is nullable - whichever of the two a
//! message states first, every member of it reading the definition as it
//! is once the message is read - the definition's `FIX:sources` lists every
//! dialect that declared it, and a later grammar's member reads the held
//! definition under the member name its own grammar gave it.
//! [`FixRegistry::merge_with`] folds two dictionaries' definitions by name
//! first and then the same way: each pair of one structure the fold made -
//! an arrival stating a held definition's structure, or a held definition
//! the fold widened into another's - folds into the held one, so a split
//! one dialect took for one message is the group it was split from as soon
//! as the fold makes the two alike, while two definitions one dictionary
//! itself states as two stay two. A record stating no member states no
//! structure, here and there alike.
//!
//! # What is lost, by name
//!
//! `part` (so a `header` constraint and a `body` constraint sit as siblings
//! in one flat struct), `activated`, `read-only`, `ref`, `checkordering`,
//! `rg-name` beyond naming its group, `condition` and every `expression`
//! beyond the one tag a mapping refers to,
//! `regexp`, `domain`, every range and sentinel, every validity element,
//! `merge-mode` (so a file patching a base configuration is read standalone),
//! a `message-type`'s `supported` and `rejection`, `history`, `cvs-revision`,
//! the root's `description`, `normalization-binding` beyond the names it
//! spells, `reject-binding`, `flow-filter-binding`, `options`,
//! `noe-normalization-binding`, and the root's own `version`, `date` and
//! `logs`.
//!
//! # What a type word means
//!
//! A `vocabulary-tag` types itself with one of eight words - `string`,
//! `char`, `integer`, `float`, `boolean`, `utc-date`, `utc-timestamp` and
//! `utc-time-only` - and seven of them resolve through the schema grammar
//! and its logical names exactly as a caller's datatype expression would,
//! folding case and separators, so `utc-date` is `utcdate`, that day's
//! midnight in UTC. `float` is the one read here for itself: the grammar
//! reads that word as SQL does, a 32-bit float, where a CBlock means FIX by
//! it - the family `Qty`, `Price` and `Amt` derive from, which states no
//! width and which this crate types float64. A word outside the eight
//! resolves through the same grammar - a real export spelling
//! `LocalMktDate`, `MonthYear`, `data` or a datatype name outright reads as
//! what it names - and a word nothing reads types the tag as text, with a
//! warning naming the word: the tag is still on the wire, every FIX datatype
//! is text there, and a tag dropped for its type word would take every
//! constraint naming it out of every message that binds it.
//!
//! The eight words are coarse, and the fold reads them as such. A CBlock's
//! `float` meets the dictionary's `decimal128(38, 18)` for `AvgPx`, its
//! `string` meets `ccy` for `Currency`, `mic` for `SecurityExchange`, `side`
//! for `Side`, `binary` for `RawData` and `fixed_ascii(8)` for
//! `MaturityMonthYear`, its `integer` meets `int64` for `MsgSeqNum`, and
//! its `utc-date` meets the zone-less date the dictionary holds for
//! `TradeDate`: each says less than the dictionary, not something else, so
//! [`FixRegistry::merge_with`] keeps the dictionary's declaration and folds
//! the file's field under it, counted in
//! [`FixMerge::restated`](super::FixMerge) rather than passed over. Only a
//! contradiction - a `boolean` against a stored `int32`, a time of day
//! against a timestamp - is named in [`FixMerge::dropped`](super::FixMerge),
//! and it never stops the rest of the file or the other files of a glob.
//! The parser itself consults no other registry and promotes no value by
//! tag. Replacing a referenced field still fails when its existing layouts
//! would become inconsistent; an unreferenced identity may be replaced
//! directly.
//!
//! A group or component one message declares with other members than another
//! message already declared under that name, and alike to no definition held
//! under any other, is split under the name the message is catalogued by -
//! `underlying_newordersingle`, or `underlying_message414e` where tag 35
//! names the type nothing a store can file - while the member the message
//! holds keeps the grammar's name. A message declaring one name in several
//! shapes - its parties at the root and again inside its legs - takes a
//! split per shape in the order it declares them, `party_message5a`,
//! `party_message5a_2`: a shape is one definition, and nothing a member has
//! already read is rewritten under it. A message the catalog will not hold
//! takes back every definition its walk wrote.
//!
//! # Names a catalog can file
//!
//! A reference names a definition by a catalog name - ASCII letters,
//! digits, `_`, `-` and `.` - and a CBlock spells freely: `OTC Trade Flags`,
//! `(BloombergCustomTag05)`, an `rg-name` of `No Fidessa Legs`. So every
//! name this reader gives - a field from its `alt` or its normalization, a
//! group and its occurrence from the `rg-name` or the counter - is the
//! catalog name the spelling folds to: lower case, every run of anything
//! else one `_`, no `_` at either end, `otc_trade_flags`. The spelling stays
//! the definition's `display`, and a spelling nothing of which folds leaves
//! a tag named by its decimal, as a tag declaring no `alt` is.
//!
//! Nothing is inferred from a validity child either: a `regexp` pinning a
//! length does not become a fixed-width ascii, and a `domain="ranges"` does
//! not become an enum.
//!
//! A description keeps its words and loses its layout. A stored description
//! holds no control character, and a CBlock wraps a long one over several
//! indented lines, so every run of whitespace and control characters becomes
//! one space and the ends are dropped - the file's line breaks are the file's
//! formatting, and losing a whole description over one of them would be
//! losing what a document nothing is wrong with says. Only the tag's own
//! `description` children describe it, escaped or in a CDATA section, and two
//! of them are two sentences: a sibling's text, and a validity child's own
//! `description`, are that element's.
//!
//! # A spelling two tags share
//!
//! A real dialect spells one `alt` over two tags. `TRTN_FX_TradeCapture`
//! declares `HedgeCurrency` twice, once for the currency a hedge settles in
//! and once for the currency it is quoted in, and the two are different
//! fields with different tags. A dictionary indexes a name once, so that
//! spelling cannot name both there - and picking either would give a
//! reader a `HedgeCurrency` the file never said was the one.
//!
//! So it names neither by it. The file's other statement of what a tag is
//! called is read first: a normalization binding that spells one of the two
//! by a name of its own - `LEGLASTSPOTRATE` for a tag 5190 whose `alt`
//! repeats tag 637's `LegLastPx`, one vendor's copy of another's line -
//! names that tag, and the spelling is then one tag's alone and stays with
//! it. A tag whose `alt` another tag also declares, and a tag whose `alt` is
//! another tag's own decimal, that the bindings leave unnamed too are named
//! by their own decimal - the identity a tag declaring no `alt` already
//! takes - and keep the declared spelling as their `display`. Contended by
//! the key a dictionary indexes a name under rather than by the spelling,
//! because that key folds case and drops `_`, `-` and space:
//! `Hedge_Currency` beside `HedgeCurrency` is one name there and has to be
//! one name here. That leaves every tag named, always, and it is the same
//! reading the normalization section below applies to a name that means a
//! tag only sometimes.
//!
//! A field named by its own decimal is unnamed, and the fold reads it so: a
//! later file naming that tag names the field, which takes the name rather
//! than standing beside a numbered twin of itself - [`FixRegistry::add_field`]
//! states the rule - so the members of both files read one field.
//!
//! The two keep each other. Each records the other's tag among its alternate
//! tags, so a reader holding either half of the pair reaches the one the file
//! said the same thing about; three tags sharing a spelling record nothing,
//! because an alternate identifier names one field and three would each claim
//! the other two.
//!
//! What restores the spelling is the message. A `tag-constraint` names one
//! tag, so a message root, a component and a group each carry the spelling at
//! the one place the file made it unambiguous, and a reader resolving a key
//! against the message it arrived in reaches the tag the file meant. That is
//! what [`FixCodec`](super::FixCodec) does with a bridge row's `MSGTYPE`: the
//! dictionary answers nothing for a shared spelling, and the message the row
//! declares answers with the tag it bound. Two constraints of one grammar
//! carrying one spelling are still two children, numbered as duplicate
//! constraints already are.
//!
//! # What a normalization spells a tag with
//!
//! A `vocabulary-tag` states what a tag *is*; a `normalization-binding`
//! states, over and over, what this counterparty *calls* it. Only the second
//! half is read, and only where the file states it plainly: a
//! `tag-normalization` whose mapping is one bare `$602` and nothing else says
//! that its `tag-name` is another spelling of tag 602, so the spelling joins
//! that tag: as its name, where the vocabulary left the tag unnamed -
//! declaring no `alt`, or an `alt` another tag also declares - and every
//! binding of the tag agrees on one spelling that no other tag's `alt`,
//! decimal or binding claims; as an alias otherwise, the vocabulary keeping
//! the name it gave.
//!
//! Everything else in the binding is read past, because everything else is a
//! computation this layer has no evaluator for. `lookup("SecurityIDSource",
//! $603)` decodes a value rather than naming a tag. A `mapping-condition`
//! makes the name conditional, and a name that means a tag only sometimes is
//! not another spelling of it: `LEGISINCODE` is `$602` under `$603 = "4"` and
//! `LEGEXCHANGECODE` is the same `$602` under `$603 = "8"`, so taking either
//! would give tag 602 two names it answers to unconditionally and neither of
//! them what the file said. Two expressions under one mapping are a
//! construction and name nothing either. Neither is `rg-name`, which names a
//! repeating group rather than the counter tag beside it - and a binding that
//! nests one spells that counter plainly in its own `tag-normalization`
//! anyway. `noe-normalization-binding` keeps its own name for the same
//! reason it keeps everything else: nothing says it spells a tag the way this
//! one does.
//!
//! Most of what a real binding spells is a name the tag already answers to,
//! and those add nothing: resolution folds ASCII case, so a
//! `tag-name="LEGSECURITYID"` over a tag the vocabulary spelled
//! `LegSecurityID` is a spelling that already resolves. What is left is what
//! this is worth reading for - a tag whose `vocabulary-tag` declared no `alt`
//! has no name but the normalization's, which is then the only place the
//! file says what the tag is called, and only a tag that leaves unnamed too
//! is named by its own decimal.
//!
//! No name is refused. A tag outside the vocabulary, a spelling another tag
//! already answers to, a spelling the core could not store: each drops that
//! one name, because a binding otherwise read past is not a place to refuse a
//! dictionary from.
//!
//! # Every message type the file names
//!
//! A CBlock states its message types three ways, and all three are read into
//! one shared code set referenced by tag 35: the `message-types` listing states the type and
//! the wording beside it, the two mapping tables spell the same type the way
//! UlMessage does, and a `grammar-binding` binds one the listing sometimes
//! omits. Each becomes one code - the wire value as its name, every spelling
//! the file gave it as aliases, the listing's wording as its description.
//!
//! A CBlock spells a type as the wire value and a qualifier, and the value is
//! the first word: `AR Inbound` and `AR Outbound` are tag 35 `AR` in the two
//! directions, `J Report` is `J` used as a report, `c SDR` and `c SLR` are
//! `c` used two ways. A FIX message type is alphanumeric and never holds a
//! space, so **the qualified spelling is a spelling and never a name**: a
//! file that binds `6 Inbound` has said that tag 35 `6` exists and what this
//! dialect calls that use of it, not what the type is called. The code is
//! therefore named after the value it is, which is a placeholder a real name
//! displaces the moment this folds into a dictionary that has one - `6`
//! becomes `ioi` and keeps `6 Inbound` among its spellings, rather than
//! renaming the type after a direction.
//!
//! **A wire value is case-bearing and the fold stops at it.** Tag 35 reads
//! `b` as MassQuoteAcknowledgement and `B` as News, `c` as
//! SecurityDefinitionRequest and `C` as Email, `d`, `g`, `h`, `j`, `q`, `r`
//! and `s` against their own capitals the same way - so a dialect stating
//! both cases of a letter states two messages, and each keeps its own
//! grammar, its own wording and its own entry. The qualifiers around it
//! still fold, because they are spellings: `b Inbound`, `b-inbound` and
//! `B Inbound` all reach whichever value they qualify. Only the value
//! itself is read exactly, which is the one reading the wire has.
//!
//! **One wire type is one message**, however many grammars the file binds
//! under it: the second binding folds into the first, keeping its members in
//! order and appending every member only the second declares, so a dialect
//! that describes a type inbound and outbound holds one message carrying
//! both. The roots the reader hands back are still one per binding, because
//! that is what the file bound. The complete wire token is retained without
//! a width limit.
//!
//! A message type is a code of tag 35 and never a field, so a file that
//! declares no tag 35 keeps its types out of the dictionary rather than
//! inventing the field to hold them. What the file's own `map` for tag 35
//! said leads, because a map states a wire value.
//!
//! # The charset
//!
//! A CBlock states its charset the way any XML document does - a byte order
//! mark, else the declaration's `encoding`, else UTF-8 - and real exports
//! declare `US-ASCII`, `UTF-8` and `ISO-8859-1`. The document crosses the
//! charset boundary once, before the XML reader sees a byte: an all-ASCII
//! document is borrowed, and anything else is decoded to the UTF-8 the rest
//! of this parser reads, so a `é` in a `description` attribute is the
//! character rather than a refusal of the whole file. Bytes the declared
//! charset cannot read are transcribed - every valid run kept, a stray byte
//! read as the Windows-1252 character it is - and a charset the crate has no
//! table for is read under UTF-8, each named once at warn level.
//!
//! # What is dropped, and what is refused
//!
//! A CBlock is read for what it says. A real one is megabytes over hundreds
//! of thousands of elements, and one element this reader cannot make sense of
//! is one element: a tag spelled in a way the core cannot store, a mapping to
//! a type nothing listed, a nested grammar with no counter. Each is dropped,
//! the rest of the file is still a dictionary, and what went is logged at
//! warn level.
//!
//! A dropped element carries the sentence a refusal would have, and then
//! what the reader did about it: one [`Error::Parse`] with `cfb` as its
//! target, the byte the reader had reached as its position, the line and
//! column that byte falls on, and a reason that quotes the document rather
//! than describing it - `line 41, column 3: expected a decimal tag, got
//! "MsgType" in "<vocabulary-tag name=\"MsgType\" type=\"string\">"` -
//! followed, after a semicolon, by the consequence, `the declaration is
//! dropped`. One that comes from the core rather than from the grammar - a
//! description or a spelling that cannot be stored, a code set that cannot be
//! rendered, two declarations of one tag - carries the core's own sentence
//! behind the tag and the wording that raised it. Every span quoted from the
//! document is bounded and the core's is bounded by the error contract it is
//! written under, so a warning never grows with the file. What each drop
//! leaves behind is what its sentence says: a tag whose wording will not
//! store keeps the tag, a tag whose type word nothing reads is typed string,
//! a constraint on a tag the vocabulary never declared declares it as text
//! named by its digits and keeps the member, a group with no counter keeps
//! its parent, and a message the catalog will not hold keeps every field the
//! file declared.
//!
//! An attribute is one attribute. One the reader cannot split into a key and
//! a quoted value is dropped and the element keeps the rest; one whose value
//! will not unescape - `S&P 500`, an HTML entity - keeps the value as the
//! file spelled it; one a hand-edited element states twice is read once, the
//! first statement standing; and a bare `&` in a description's text is the
//! character. Each is named the same way, and none refuses the document.
//!
//! A map entry whose key or value spells nothing - `none`, `null`, blank -
//! states no code: a venue writes one where it has none to state, so it is
//! skipped, never claims a name and never meets a code the dictionary
//! holds, and the entries one code set skipped are named in one warning.
//!
//! Two things are refusals, because neither leaves anything to keep. A
//! document that is not XML is one: the reader stopped, and it quotes the
//! bytes just before it did, because there is no element to name. A document
//! that stops inside an element - a partial download, a half-written file -
//! is the other, naming the element it left open; the reader answers no error
//! for one, so each loop that reads to its own end tag says so itself. Only
//! the top-level loop's end of document is an ending, so a file cut between
//! two of the root's children reads as what it holds rather than as a defect:
//! nothing was left open but the root.
//!
//! The vocabulary is checked against itself after the document is read, and
//! each declaration keeps the byte it was read at so that a warning names its
//! own tag's declaration rather than the end of the file.

use std::borrow::Cow;
use std::fmt::Display;

use quick_xml::Reader;
use quick_xml::events::{BytesStart, Event};
use smol_str::{SmolStr, format_smolstr};

use crate::implementer::{ERROR_TEXT_LIMIT, elide_to, expected_got};
use crate::{Charset, DataType, Error, Field, IOBase, Result, Side, StructType, Url};

use super::catalog::{catalog_name, push_member};
use super::codes::{FixCodes, claims, is_sentinel, names_collide};
use super::{FixCode, FixField, FixFieldMut, FixRegistry, MSGTYPE_TAG_NAME};

/// How deep a grammar may nest before the parse stops descending.
///
/// The deepest observed in production is five. The guard is well above that
/// and exists so a malformed or hostile file cannot recurse the stack away.
const MAX_DEPTH: usize = 32;

/// What this parser names itself in a refusal and in a warning.
const TARGET: &str = "cfb";

/// The eight types a `vocabulary-tag` declares, for a warning to name.
///
/// Seven resolve straight through the schema grammar and its logical names,
/// which fold case and drop separators - `utc-date` folds to `utcdate`, a
/// spelling the table carries beside `utcdateonly`. `float` is the one
/// [`cblock_type`] reads for itself, because the grammar reads that word as
/// SQL does and a CBlock means FIX by it.
const TYPES: [&str; 8] = [
    "string",
    "char",
    "integer",
    "float",
    "boolean",
    "utc-date",
    "utc-timestamp",
    "utc-time-only",
];

/// How much of a document the charset declaration is looked for in.
///
/// An XML declaration is the first line of the document, and a CBlock's is
/// under a hundred bytes; the bound keeps the probe from scanning a document
/// that opens with none.
const DECLARATION_PROBE: usize = 512;

/// What every drop of a nested grammar leaves behind, for the warning to say.
const GROUP_DROPPED: &str = "the group is dropped and the message keeps the rest";

/// How many skipped map entries one warning lists before it counts the rest.
const SKIPPED_LISTED: usize = 16;

/// The two domains a validity element may declare.
const DOMAINS: [&str; 2] = ["all-values", "ranges"];

impl FixRegistry {
    /// Parses an Ullink CBlock configuration into the vocabulary it declares
    /// and the message roots its grammar bindings describe.
    ///
    /// `dialect` names the dictionary, because a `.cfb` never names itself:
    /// the file states a version and a session but no name for the pair, so
    /// the caller supplies one, every field the file produces is stamped as
    /// a member of it in its `FIX:sources`, and the dictionary holds the
    /// dialect's entry in its sources catalog, naming the file where the
    /// handle names one and the role of the plugin the root's `type` names.
    /// `None` stamps nothing and holds no entry, which is
    /// right for a file read only for its vocabulary. The root element's
    /// `fix-version`, `sendercompid` and `targetcompid` are read past: which
    /// version a run reads at is the codec's pin.
    ///
    /// The registry holds scalar wire fields under their tags and message
    /// roots as components carrying `FIX:msgtype`. Nested grammars contribute
    /// Groups and Components definitions, with `FIX:counter` linking each
    /// serie to its ordinary int32 field, which no message lists beside the
    /// serie. Enumerations name the registry set in `FIX:codeset`
    /// metadata. Named definitions carry no `FIX:tag`.
    ///
    /// No seed is taken: this answers what one file says. Folding it into a
    /// dictionary that already exists is [`FixRegistry::add_cfb_file`]'s job,
    /// which is this parse and [`FixRegistry::merge_with`] in one call, so
    /// the code sets the file declares travel with the fields naming them.
    ///
    /// `dialect` names the dictionary, and nothing stands in for it: this
    /// door answers what one file says, so a file read with no name stamps no
    /// membership. The stem stands in where a file is *folded* into a
    /// dictionary - [`FixRegistry::add_cfb_file`] and
    /// [`FixRegistry::add_cfb_files`] - because that is where a contribution
    /// has to be attributable and a `CBlock` never names itself.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Parse`] naming the byte position and quoting what the
    /// reader stopped on, for a document that is not well-formed XML or that
    /// stops with an element open. Everything else this reader cannot make
    /// sense of - a `type` outside the eight, a
    /// `domain` outside the two, a constraint whose tag misses the vocabulary,
    /// a nested grammar with no counter, nesting past the guard depth, a
    /// mapping to a type nothing listed, and anything the core will not store,
    /// from a spelling to a second declaration of one tag - is dropped with a
    /// warning naming it, and the rest of the file is still a dictionary. The
    /// warnings are `log` records at warn level; nothing is emitted unless the
    /// host installs a logger.
    ///
    /// What the file states twice is not one of them. A type the listing and
    /// a binding both declare, a grammar bound under a wire type another
    /// grammar already bound, and a member a held message already carries are
    /// each what a dialect looks like rather than a defect: the declarations
    /// fold, the members union, and the fold is recorded at info level where
    /// it is recorded at all.
    pub fn from_cfb_file(handle: &dyn IOBase, dialect: Option<&str>) -> Result<(Self, Vec<Field>)> {
        parse(&handle.read_all_bytes()?, dialect, handle.url())
    }
}

/// One document's bytes, read the way [`FixRegistry::from_cfb_file`] reads
/// them: what the parse needs is the bytes and nothing else, so a caller that
/// already holds them - a fold spreading its files over threads - parses them
/// where it holds them.
pub(super) fn parse(
    bytes: &[u8],
    dialect: Option<&str>,
    source: Option<&Url>,
) -> Result<(FixRegistry, Vec<Field>)> {
    let prefix = warning_prefix(source, dialect);
    let text = decoded(bytes, &prefix);
    Parse::new(text.as_bytes(), dialect, source_file_name(source), prefix)?.run()
}

/// The name of the file a parse reads, as the file is named: the last
/// segment of its URL, percent escapes decoded, and nothing for a buffer,
/// whose identity is an address rather than a location.
fn source_file_name(source: Option<&Url>) -> Option<Cow<'_, str>> {
    let name = source
        .filter(|url| !url.to_string().starts_with("mem:"))
        .and_then(Url::file_name)?;
    Some(crate::implementer::percent_decode(name, "url path").unwrap_or(Cow::Borrowed(name)))
}

/// What every warning this parse logs opens with: the file's name and the
/// dialect, each where there is one - `venue.cfb [venue] ` - so a warning
/// among the thousands a glob of files logs side by side says which file it
/// is about. A buffer's identity is an address rather than a location, and
/// names nothing.
fn warning_prefix(source: Option<&Url>, dialect: Option<&str>) -> String {
    let mut prefix = String::new();
    if let Some(name) = source_file_name(source) {
        prefix.push_str(&name);
        prefix.push(' ');
    }
    if let Some(dialect) = dialect {
        prefix.push('[');
        prefix.push_str(dialect);
        prefix.push_str("] ");
    }
    prefix
}

/// The line and column `position` falls on in `bytes`, one-based.
fn line_column(bytes: &[u8], position: usize) -> (usize, usize) {
    let position = position.min(bytes.len());
    let before = &bytes[..position];
    let line = before.iter().filter(|byte| **byte == b'\n').count() + 1;
    let start = before
        .iter()
        .rposition(|byte| *byte == b'\n')
        .map_or(0, |newline| newline + 1);
    (line, position - start + 1)
}

/// The document as UTF-8 text, decoded once from the charset it declares.
///
/// A CBlock states its charset the way any XML document does - a byte order
/// mark, else the declaration's `encoding`, else UTF-8 - and real exports
/// declare `US-ASCII`, `UTF-8` and `ISO-8859-1`. The XML reader beneath this
/// parser reads UTF-8 alone: a byte that is not UTF-8 inside an attribute
/// value would refuse the whole document, and inside a description would be
/// read as a replacement character. So the text crosses the charset boundary
/// here, once, as every byte payload in this crate does, and everything past
/// this point reads `str`. An all-ASCII document, which is every CBlock but a
/// description or two, is borrowed; a refusal's byte position past this point
/// is a position in the decoded text, which is the document's own wherever
/// the document was ASCII.
///
/// Bytes the declared charset cannot read are transcribed rather than
/// refused, every valid run kept and a stray byte read as the Windows-1252
/// character it is, and named once with the byte, the line and the column
/// the decoder stopped at, because a dictionary of thousands of fields is not
/// refused over the one byte of a description. A charset the crate has no
/// table for is named the same way and the document read under UTF-8, which
/// is what a CBlock's grammar is written in whatever its descriptions hold.
fn decoded<'bytes>(bytes: &'bytes [u8], prefix: &str) -> Cow<'bytes, str> {
    let warn = |position: usize, reason: SmolStr, kept: &str| {
        let (line, column) = line_column(bytes, position);
        log::warn!(
            "{prefix}{}; {kept}",
            refusal(
                position,
                format_smolstr!("line {line}, column {column}: {reason}")
            )
        );
    };
    let (charset, body) = match Charset::from_bom(bytes) {
        Some((charset, mark)) => (Ok(charset), &bytes[mark..]),
        None => {
            let head = &bytes[..bytes.len().min(DECLARATION_PROBE)];
            let declared =
                crate::implementer::declared_charset(head).map(Option::unwrap_or_default);
            (declared, bytes)
        }
    };
    let charset = charset.unwrap_or_else(|error| {
        warn(
            0,
            format_smolstr!("the document's charset: {error}"),
            "read as utf-8",
        );
        Charset::Utf8
    });
    match charset.decode(body) {
        Ok(text) => text,
        Err(error) => {
            let position = match &error {
                Error::Codec { position, .. } => *position,
                _ => 0,
            };
            warn(
                position,
                format_smolstr!("the document's {charset} text: {error}"),
                &format!("every byte that is not {charset} is read as Windows-1252"),
            );
            charset.transcribe(body)
        }
    }
}

/// The dialect a handle's own stem names, where it names one.
///
/// A stem stands in for a name the caller did not supply, so it is taken
/// only where it reads as a source id: opening with an ASCII letter and
/// holding no quote, backslash or control character. The address a buffer
/// is identified by, a bare number and a stem the id grammar refuses are
/// stems and not names, and stand in for nothing - the file reads as a bare
/// vocabulary rather than being refused for a name nobody supplied. A stem
/// is read as the file is named rather than as its URL spells it, so
/// `Morgan Stanley.cfb` names `morgan stanley` and not the percent escape
/// in between.
pub(super) fn stem_dialect(handle: &dyn IOBase) -> Option<Cow<'_, str>> {
    let stem = handle.url().and_then(Url::stem)?;
    let stem = crate::implementer::percent_decode(stem, "url path").unwrap_or(Cow::Borrowed(stem));
    let named = stem
        .bytes()
        .next()
        .is_some_and(|byte| byte.is_ascii_alphabetic())
        && super::document::is_word(&stem);
    named.then_some(stem)
}

/// One file being read.
struct Parse<'doc> {
    reader: Reader<&'doc [u8]>,
    /// The document itself, for the text a warning over it quotes.
    ///
    /// The reader answers where it stopped and not what it stopped on, and a
    /// malformed document has no element to name, so the bytes are kept to
    /// quote the span just before that position.
    bytes: &'doc [u8],
    /// The dialect every produced field is a member of - its catalog entry,
    /// the id as the caller named it folded, and the file's name where the
    /// handle names one - and nothing, for a file read as a bare vocabulary.
    source: Option<super::source::FixSource>,
    /// The role of the plugin the root element's `type` names - its class,
    /// `BuySideFIXCPluginCBlock` or `SellSideFIXCPluginCBlock` - which the
    /// dialect's catalog entry states; `UKNW` until the root is read, and
    /// where it names neither.
    pluginside: Side,
    /// The vocabulary in declaration order, and where each tag sits in it.
    ///
    /// Indexed rather than scanned: a binding resolves every constraint it
    /// carries against the whole vocabulary, and both are thousands long in a
    /// real file - so a scan per constraint is the parse squared.
    vocabulary: Vec<Declared>,
    positions: std::collections::HashMap<i32, usize>,
    /// The message types the file declares, in declaration order.
    ///
    /// A CBlock states them three ways - a `message-types` listing, the two
    /// mapping tables that spell them the way UlMessage does, and the
    /// `grammar-binding` that binds one - and each is a type this dialect
    /// carries. They are collected rather than written as they arrive because
    /// the listing precedes the `vocabulary` that declares tag 35.
    msgtypes: Vec<FixCode>,
    /// Where each message type sits, by wire value and by folded spelling.
    ///
    /// Indexed rather than scanned, for the reason the vocabulary is: a file
    /// lists a type per message and spells each of them twice more, and a
    /// scan per declaration is the parse squared.
    values: std::collections::HashMap<SmolStr, usize>,
    spellings: std::collections::HashMap<String, usize>,
    /// Each message root, beside the position of the `grammar-binding` that
    /// declared it, which names the binding where the catalog drops it.
    roots: Vec<(Field, usize)>,
    /// The names the normalization bindings spell for a tag, in declaration
    /// order.
    ///
    /// Collected rather than attached as they arrive, for the reason the
    /// message types are: a normalization is checked against the whole
    /// vocabulary - the tag it names, what that tag is already called, and
    /// what every other tag answers to - and a file is free to spell one
    /// before it declares the other.
    named: Vec<Spelled>,
    /// Where each line of the document starts, indexed on the first warning
    /// so a warning can name the line beside the byte.
    lines: std::cell::OnceCell<Vec<usize>>,
    /// What every warning opens with: the file's name and the dialect, each
    /// where there is one ([`warning_prefix`]).
    prefix: String,
}

/// One name a `tag-normalization` stated for a tag.
struct Spelled {
    tag: i32,
    /// The name as the file spelled it, which is the spelling stored.
    name: String,
    /// The byte the reader had reached, so a warning names this element
    /// rather than the end of the file.
    position: usize,
}

/// One `vocabulary-tag` as the file declared it.
struct Declared {
    tag: i32,
    /// The code set this tag's maps decode, held apart from the field.
    ///
    /// A field names its set and the dictionary owns the members, so the
    /// members are collected here while the file is read and filed under the
    /// field's own name when the dictionary is built.
    codes: Vec<FixCode>,
    /// The byte the reader had reached when this declaration was read.
    ///
    /// Kept because the dictionary is built after the whole document is: a
    /// warning over two declarations of one tag names the declaration that
    /// raised it rather than the end of the file.
    position: usize,
    /// The map entries decoding this tag that spell no code - a key or a
    /// value that is `none`, `null` or nothing - in the order the maps
    /// stated them, named once for the whole code set when the dictionary is
    /// built.
    skipped: Vec<Skipped>,
    field: Field,
}

/// One map entry as the file spelled it: its `key` and its `value`.
type Entry = (String, String);

/// One map entry skipped because one of its two attributes spells nothing.
struct Skipped {
    /// The map the entry was read from, as the file named it.
    map: String,
    key: String,
    value: String,
    /// The byte the reader had reached at the map, for the warning to name.
    position: usize,
}

impl<'doc> Parse<'doc> {
    /// Opens a read, proving the dialect name a source id before a byte is
    /// read: a name no field could carry refuses the file, not each field.
    fn new(
        bytes: &'doc [u8],
        dialect: Option<&str>,
        file: Option<Cow<'_, str>>,
        prefix: String,
    ) -> Result<Self> {
        let source = match dialect {
            Some(name) => {
                let source = super::source::FixSource::new(name)?;
                Some(match file {
                    Some(file) => source.with_file(file.as_ref()),
                    None => source,
                })
            }
            None => None,
        };
        // Text arrives exactly as the file wrote it. The reader's own trimming
        // is per event, and a description carrying an entity arrives as
        // several, so it would eat the space in front of `&lt;SOH&gt;` and
        // join two words. Layout is [`single_line`]'s question, and the only
        // text this reads is a description's. A bare `&` in that text - `S&P
        // 500`, typed into a description by hand - is the character and not
        // a reference left open, so the document is not refused over it.
        let mut reader = Reader::from_reader(bytes);
        reader.config_mut().allow_dangling_amp = true;
        Ok(Self {
            reader,
            bytes,
            source,
            pluginside: Side::Unknown,
            vocabulary: Vec::new(),
            positions: std::collections::HashMap::new(),
            msgtypes: Vec::new(),
            values: std::collections::HashMap::new(),
            spellings: std::collections::HashMap::new(),
            roots: Vec::new(),
            named: Vec::new(),
            lines: std::cell::OnceCell::new(),
            prefix,
        })
    }

    /// Stamps one produced field as a member of this file's dialect.
    fn stamp(&self, field: &mut Field) -> Result<()> {
        match &self.source {
            Some(source) => FixFieldMut::new(field).set_sources([source.id()]),
            None => Ok(()),
        }
    }

    /// The vocabulary as a dictionary, and the roots the bindings describe.
    fn run(mut self) -> Result<(FixRegistry, Vec<Field>)> {
        self.read()?;
        self.dictionary()
    }

    /// The vocabulary as a dictionary.
    ///
    /// Where the file's own entries are checked against each other rather
    /// than only against the grammar, and where each map's code set is filed
    /// under the name the field that decodes by it supplies.
    fn dictionary(mut self) -> Result<(FixRegistry, Vec<Field>)> {
        let mut registry = FixRegistry::new();
        // The dialect's entry, so no field the file produces names an id
        // the dictionary holds no entry for, stating the role the root
        // named. Cloned, not taken: the roots catalogued below are stamped
        // from the same entry.
        if let Some(source) = &self.source {
            registry.add_source(source.clone().with_pluginside(self.pluginside));
        }
        for held in std::mem::take(&mut self.vocabulary) {
            let Declared {
                tag,
                position,
                codes,
                skipped,
                mut field,
            } = held;
            // Read before the field moves: the warning names the entry the
            // file declared, which the registry no longer has to hand.
            let named = SmolStr::new(field.name());
            if !skipped.is_empty() {
                self.skipped_entries(tag, &field, &skipped);
            }
            // The set the file's maps decode, filed under the name this field
            // supplies and named by the field: the dictionary owns the
            // members, so they are stated before the field points at them.
            let mut codeset_checkpoint = None;
            if !codes.is_empty() {
                let set = FixRegistry::derived_codeset_name(&field);
                let result: Result<()> = (|| {
                    let checkpoint = registry.codeset_checkpoint(&set)?;
                    registry.set_codeset(&set, &codes)?;
                    if let Err(error) = FixFieldMut::new(&mut field).set_codeset(&set) {
                        registry.restore_codeset(checkpoint.0, checkpoint.1);
                        return Err(error);
                    }
                    codeset_checkpoint = Some(checkpoint);
                    Ok(())
                })();
                if let Err(error) = result {
                    self.dropped(
                        &self.refusal_at(
                            position,
                            format_smolstr!(
                                "the code set tag {tag} {:?} decodes: {error}",
                                elide_to(&named, ERROR_TEXT_LIMIT)
                            ),
                        ),
                        "the field keeps no code set",
                    );
                }
            }
            if let Err(error) = registry.insert(field) {
                if let Some((key, previous)) = codeset_checkpoint {
                    registry.restore_codeset(key, previous);
                }
                self.dropped(
                    &self.refusal_at(
                        position,
                        format_smolstr!(
                            "tag {tag} {:?}: {error}",
                            elide_to(&named, ERROR_TEXT_LIMIT)
                        ),
                    ),
                    "the declaration is dropped",
                );
            }
        }
        let mut roots = Vec::with_capacity(self.roots.len());
        let held = std::mem::take(&mut self.roots);
        let mut structures = Structures::default();
        for (root, at) in held {
            let wire = super::msgtype::wire_value(root.name()).to_owned();
            let named = SmolStr::new(root.name());
            match self.catalogued(&mut registry, root, &wire, &mut structures) {
                Ok(root) => roots.push(root),
                // One message the catalog will not hold is one message: the
                // dictionary keeps every field it read and every other root
                // the file bound.
                Err(error) => self.dropped(
                    &self.refusal_at(
                        at,
                        format_smolstr!(
                            "message {:?}: {error}",
                            elide_to(&named, ERROR_TEXT_LIMIT)
                        ),
                    ),
                    "the message is dropped and every field it declared kept",
                ),
            }
        }
        Ok((registry, roots))
    }

    /// Names, in one warning, every map entry decoding `tag` that was
    /// skipped because it spells no code: one line per code set however many
    /// maps and entries it covers, so a file of placeholder entries reads as
    /// one sentence per vocabulary rather than one per entry. The entries are
    /// listed up to [`SKIPPED_LISTED`], the rest counted.
    fn skipped_entries(&self, tag: i32, field: &Field, skipped: &[Skipped]) {
        let mut listed = String::new();
        for entry in skipped.iter().take(SKIPPED_LISTED) {
            if !listed.is_empty() {
                listed.push_str(", ");
            }
            listed.push_str(&format!(
                "{:?} key {:?} value {:?}",
                elide_to(&entry.map, ERROR_TEXT_LIMIT),
                elide_to(&entry.key, ERROR_TEXT_LIMIT),
                elide_to(&entry.value, ERROR_TEXT_LIMIT)
            ));
        }
        if skipped.len() > SKIPPED_LISTED {
            listed.push_str(&format!(" and {} more", skipped.len() - SKIPPED_LISTED));
        }
        let count = skipped.len();
        self.dropped(
            &self.refusal_at(
                skipped[0].position,
                format_smolstr!(
                    "tag {tag} {:?}, code set {}: {count} map {} spelling none, null or nothing: {listed}",
                    elide_to(spelled(field), ERROR_TEXT_LIMIT),
                    FixRegistry::derived_codeset_name(field),
                    if count == 1 { "entry" } else { "entries" },
                ),
            ),
            "an entry spelling nothing states no code, so each is skipped",
        );
    }

    /// One root as the catalog holds it: its name resolved through tag 35's
    /// own code set, its members registered under that name, and the entry
    /// itself written.
    ///
    /// **One wire type is one message, however many grammars the file binds
    /// under it.** A name here is derived from the wire value alone, so `6
    /// Inbound` and `6 Outbound` reach the same entry, and the second one
    /// *folds* into the first rather than being qualified into a message of
    /// its own: the members the first declared stay, in their order, and
    /// every member only the second declares is appended. A member the two
    /// declare differently is read as a fold with another dictionary reads
    /// one: two groups on one counter fold their members together, a group
    /// on another counter stands beside, and only what neither reading holds
    /// is dropped, with a warning, while the rest of the binding folds. The
    /// two roots the caller is handed are still one per binding - that is
    /// what the file bound - but the dictionary holds the union, which is the
    /// message the dialect actually speaks.
    ///
    /// **The name is settled before a member is.** A group or component
    /// another context already declared alike, under its name or any other,
    /// is that definition, and the member reads it by the held name. One
    /// whose name another context holds with other members, alike to nothing
    /// held, is split under this message's name - `underlying_newordersingle`,
    /// or `underlying_message414e` where tag 35 names the type nothing
    /// readable - so a split definition says which message it came from; a
    /// third shape in one message takes `_2`, a fourth `_3`.
    fn catalogued(
        &self,
        registry: &mut FixRegistry,
        root: Field,
        wire: &str,
        structures: &mut Structures,
    ) -> Result<Field> {
        // One message is one mutation of the file's dictionary: the groups
        // and components its walk wrote go with it where it is dropped, so a
        // message the catalog will not hold leaves no definition nothing
        // reads.
        let mut written = Vec::new();
        let catalogued = self.catalogue(registry, root, wire, &mut written, structures);
        if catalogued.is_err() {
            registry.forget_definitions(&written);
            structures.forgotten(&written);
        }
        catalogued
    }

    /// [`Self::catalogued`]'s walk, recording every definition it writes.
    fn catalogue(
        &self,
        registry: &mut FixRegistry,
        root: Field,
        wire: &str,
        written: &mut Vec<(crate::FixCategory, String)>,
        structures: &mut Structures,
    ) -> Result<Field> {
        let qualifier = message_name(registry, wire);
        let mut root = catalog_members(registry, root, &qualifier, written, structures)?;
        // The root a caller is handed is the file's too, so it carries the
        // membership the catalogued message carries; only its name differs,
        // the caller's keeping the wire's spelling.
        self.stamp(&mut root)?;
        let held = root.clone();
        // Asked again rather than reused: a member the file named exactly as
        // this message is now a component of that name, and the message is
        // not the member.
        root.set_name(message_name(registry, wire));
        FixFieldMut::new(&mut root).set_msgtype(wire)?;
        if registry
            .get_definition(crate::FixCategory::Components, root.name())
            .is_some()
        {
            // One wire type is one message: the grammar bound second folds
            // into the first, keeping the members already declared in their
            // order and appending every member only this binding states. A
            // member it declares otherwise is read the way a fold with
            // another dictionary reads one - two references to one group on
            // one counter fold their members together, a group on another
            // counter stands beside under a name of its own - and only what
            // cannot be read either way is passed over, named, while the
            // rest of the binding still folds.
            log::info!(
                "folding another grammar for message type {wire:?} into {:?}",
                root.name()
            );
            let mut staged = registry.clone();
            let mut drops = Vec::new();
            staged.fold_definition_into(crate::FixCategory::Components, root, Some(&mut drops))?;
            // What this binding's walk split for the message folded into what
            // the message reads, so a split nothing reads is no definition.
            staged.forget_unread(written);
            *registry = staged;
            // The fold moved members into definitions the catalog holds, so
            // what structure each states is read again when next asked.
            structures.moved();
            for drop in drops {
                self.dropped(
                    &self.refusal_at(
                        0,
                        format_smolstr!(
                            "message {:?}, bound again: {}",
                            elide_to(wire, ERROR_TEXT_LIMIT),
                            drop.reason
                        ),
                    ),
                    "the member is dropped and the message keeps the rest",
                );
            }
        } else {
            catalog_entry(
                registry,
                crate::FixCategory::Components,
                root,
                &qualifier,
                written,
                structures,
            )?;
        }
        Ok(held)
    }

    /// Reads the whole document, skipping everything but the two passes.
    fn read(&mut self) -> Result<()> {
        let mut buffer = Vec::new();
        loop {
            match self.reader.read_event_into(&mut buffer) {
                Err(error) => return Err(self.malformed(&error.to_string())),
                Ok(Event::Eof) => break,
                Ok(Event::Start(element)) => {
                    // The root element opens the document and states its
                    // version and session, neither of which is a vocabulary's:
                    // only the plugin's class is read off it - its role is the
                    // dialect's - and its children are what is read.
                    if is_named(&element, b"cplugin-configuration") {
                        self.pluginside = self
                            .attribute(&element, "type")
                            .map_or(Side::Unknown, |class| super::plugin_side(&class));
                    } else if is_named(&element, b"vocabulary") {
                        self.read_vocabulary()?;
                    } else if is_named(&element, b"grammar-binding") {
                        self.read_binding(&element)?;
                    } else if is_named(&element, b"maps") {
                        self.read_maps()?;
                    } else if is_named(&element, b"message-types") {
                        self.read_message_types()?;
                    } else if is_named(&element, b"inbound-message-type-mappings")
                        || is_named(&element, b"outbound-message-type-mappings")
                    {
                        let name = local_name(&element);
                        self.read_msgtype_mappings(&name)?;
                    } else if is_named(&element, b"normalization-binding") {
                        self.read_normalization_binding()?;
                    } else {
                        // Skipped siblings are routinely deep and text-bearing,
                        // so this counts depth rather than assuming a child set.
                        let name = local_name(&element);
                        self.skip(&name)?;
                    }
                }
                Ok(_) => {}
            }
            buffer.clear();
        }
        self.settle_names()?;
        self.attach_msgtypes()?;
        self.attach_names()
    }

    /// Settles what each declared tag is named, once the whole file is read.
    ///
    /// A spelling that does not name exactly one tag names none of them. A
    /// tag whose `alt` another tag also declares, and a tag whose `alt` is
    /// another tag's own decimal, are therefore not named by it; a tag
    /// declaring no `alt` was never named by the vocabulary at all. What the
    /// normalization bindings call such a tag names it - the file's other
    /// statement of what a tag is called, read where the vocabulary made
    /// none, and only where it is one spelling every binding of the tag
    /// agrees on that no other tag's `alt`, decimal or binding claims. A tag
    /// both leave unnamed is named by its own decimal - the identity a tag
    /// declaring no `alt` already takes - and keeps a contended `alt` as its
    /// `display`, so nothing the file said is lost and [`spelled`] still
    /// answers what the counterparty calls it. A contended spelling that one
    /// tag alone still claims, once the others have taken the names their
    /// bindings spell, stays with that tag.
    ///
    /// Contended across the whole file, because a name is unique in the one
    /// namespace a dictionary is.
    ///
    /// Every tag is left named, always. A tag that keeps its spelling holds
    /// one no other tag claims and that is no other tag's decimal; a tag that
    /// falls back holds its own decimal, which names one tag by construction.
    ///
    /// Run once the document is read, so it settles the dictionary alone. A
    /// `tag-constraint` resolves its tag against the vocabulary while the
    /// grammar is read, and a message root's children therefore keep the
    /// spelling - which is exactly where a spelling two tags share is still
    /// unambiguous, because the file bound one tag per constraint.
    fn settle_names(&mut self) -> Result<()> {
        // The tags each name claims, in declaration order and each once, keyed
        // by what the dictionary indexes a name under rather than by the
        // spelling: the fold drops `_`, `-` and space, so `Hedge_Currency`
        // contends with `HedgeCurrency` there and has to contend here.
        let mut claimed: std::collections::HashMap<u64, Vec<i32>> =
            std::collections::HashMap::new();
        let mut decimals: std::collections::HashMap<u64, i32> = std::collections::HashMap::new();
        for held in &self.vocabulary {
            let tags = claimed
                .entry(super::registry::name_key(held.field.name()))
                .or_default();
            if !tags.contains(&held.tag) {
                tags.push(held.tag);
            }
            decimals.insert(
                super::registry::name_key(&format_smolstr!("{}", held.tag)),
                held.tag,
            );
        }
        // What the bindings call each tag: the one spelling every binding of
        // it agrees on, or nothing where they disagree or say nothing; and how
        // many tags each such spelling is spoken for, because a spelling two
        // tags are called by names neither.
        // Keyed by the catalog name each spelling folds to, the name a tag
        // would take, so two spellings one name reaches contend however they
        // are punctuated; a spelling nothing of which is a catalog name names
        // nothing.
        let folded_key = |spelling: &str| {
            catalog_name(spelling).map(|folded| super::registry::name_key(&folded))
        };
        let mut spoken: std::collections::HashMap<i32, Option<String>> =
            std::collections::HashMap::new();
        for held in &self.named {
            if held.name.contains(super::field::SEPARATOR) {
                continue;
            }
            let Some(key) = folded_key(&held.name) else {
                continue;
            };
            spoken
                .entry(held.tag)
                .and_modify(|prior| {
                    if prior
                        .as_deref()
                        .is_some_and(|prior| folded_key(prior) != Some(key))
                    {
                        *prior = None;
                    }
                })
                .or_insert_with(|| Some(held.name.clone()));
        }
        let mut speakers: std::collections::HashMap<u64, usize> = std::collections::HashMap::new();
        for key in spoken
            .values()
            .flatten()
            .filter_map(|spelling| folded_key(spelling))
        {
            *speakers.entry(key).or_default() += 1;
        }
        let mut warnings: Vec<(usize, SmolStr, &str)> = Vec::new();
        // The tags the vocabulary left unnamed take what the bindings call
        // them, where that is free.
        for at in 0..self.vocabulary.len() {
            let tag = self.vocabulary[at].tag;
            let key = super::registry::name_key(self.vocabulary[at].field.name());
            let unnamed = super::registry::is_unnamed(&self.vocabulary[at].field);
            let contended = claimed.get(&key).is_some_and(|tags| tags.len() > 1)
                || decimals.get(&key).is_some_and(|held| *held != tag);
            if !unnamed && !contended {
                continue;
            }
            let Some(Some(spelling)) = spoken.get(&tag).cloned() else {
                continue;
            };
            let Some(lowered) = catalog_name(&spelling) else {
                continue;
            };
            let spoken_key = super::registry::name_key(&lowered);
            if claimed.contains_key(&spoken_key)
                || decimals.contains_key(&spoken_key)
                || speakers.get(&spoken_key) != Some(&1)
            {
                continue;
            }
            let position = self.vocabulary[at].position;
            let field = &mut self.vocabulary[at].field;
            if lowered == spelling {
                field.remove_metadata("display");
            } else if let Err(error) = field.set_display(spelling.as_str()) {
                warnings.push((
                    position,
                    format_smolstr!(
                        "tag {tag} spelling {:?}: {error}",
                        elide_to(&spelling, ERROR_TEXT_LIMIT)
                    ),
                    "the tag keeps its decimal name",
                ));
                continue;
            }
            field.set_name(lowered.as_str());
            if let Some(tags) = claimed.get_mut(&key) {
                tags.retain(|held| *held != tag);
            }
            claimed.entry(spoken_key).or_default().push(tag);
        }
        // What is still contended is named by its own decimal.
        for at in 0..self.vocabulary.len() {
            let tag = self.vocabulary[at].tag;
            let key = super::registry::name_key(self.vocabulary[at].field.name());
            let sharing = claimed.get(&key).cloned().unwrap_or_default();
            let contended =
                sharing.len() > 1 || decimals.get(&key).is_some_and(|held| *held != tag);
            if !contended {
                continue;
            }
            let position = self.vocabulary[at].position;
            let spelling = SmolStr::new(spelled(&self.vocabulary[at].field));
            let named = format_smolstr!("{tag}");
            let field = &mut self.vocabulary[at].field;
            if spelling != named
                && let Err(error) = field.set_display(spelling.as_str())
            {
                warnings.push((
                    position,
                    format_smolstr!(
                        "tag {tag} spelling {:?}: {error}",
                        elide_to(&spelling, ERROR_TEXT_LIMIT)
                    ),
                    "the tag is named by its decimal and keeps no display",
                ));
            }
            field.set_name(named);
            // Two tags sharing a spelling name each other, so a reader holding
            // either reaches the other one the file said the same thing about.
            // Three cannot: an alternate identifier names one field, and three
            // would each claim the other two - so a spelling three tags share
            // links none of them, exactly as it names none of them.
            if sharing.len() == 2 {
                let other = sharing[usize::from(sharing[0] == tag)];
                if let Err(error) = FixFieldMut::new(field).set_tags(&[other]) {
                    warnings.push((
                        position,
                        format_smolstr!("tag {tag} beside tag {other}: {error}"),
                        "the two tags are not linked",
                    ));
                }
            }
        }
        for (position, reason, kept) in warnings {
            self.dropped(&self.refusal_at(position, reason), kept);
        }
        Ok(())
    }

    /// Reads every `vocabulary-tag` into a dictionary field.
    fn read_vocabulary(&mut self) -> Result<()> {
        let mut buffer = Vec::new();
        let mut depth = 0_usize;
        loop {
            let event = self
                .reader
                .read_event_into(&mut buffer)
                .map_err(|error| self.malformed(&error.to_string()))?;
            match event {
                Event::Eof => return Err(self.unclosed("vocabulary")),
                Event::Start(element) if is_named(&element, b"vocabulary-tag") => {
                    let element = element.into_owned();
                    let described = self.read_description()?;
                    self.push_tag(&element, described.as_deref())?;
                }
                Event::Empty(element) if is_named(&element, b"vocabulary-tag") => {
                    let element = element.into_owned();
                    self.push_tag(&element, None)?;
                }
                Event::Start(_) => depth += 1,
                Event::End(element) => {
                    if is_named(&element, b"vocabulary") && depth == 0 {
                        break;
                    }
                    depth = depth.saturating_sub(1);
                }
                _ => {}
            }
            buffer.clear();
        }
        Ok(())
    }

    /// Reads the optional `description` child of a `vocabulary-tag`.
    ///
    /// `<description />` contributes no key rather than an empty one: an empty
    /// description is the file saying nothing, and a stored empty string would
    /// be this parser saying something. A description of nothing but layout -
    /// a line break and an indent - says the same nothing.
    ///
    /// Only the text inside `description` is taken. A `vocabulary-tag` may
    /// carry other text-bearing children, and folding their words into the
    /// description would be this parser inventing a sentence the file never
    /// wrote.
    fn read_description(&mut self) -> Result<Option<String>> {
        let mut buffer = Vec::new();
        // Accumulated rather than assigned: a reader splits text at every
        // entity boundary, so one description carrying `&lt;SOH&gt;` arrives
        // as several events and keeping the last would keep a fragment.
        let mut described = String::new();
        // The tag's own children, counted, because `description` is a name a
        // validity child may carry too: only the tag's own is the tag's, and a
        // deeper one describes whatever declared it.
        let mut depth = 0_usize;
        let mut describing = false;
        loop {
            let event = self
                .reader
                .read_event_into(&mut buffer)
                .map_err(|error| self.malformed(&error.to_string()))?;
            match event {
                Event::Eof => return Err(self.unclosed("vocabulary-tag")),
                Event::Start(element) => {
                    if depth == 0 && is_named(&element, b"description") {
                        // A second description is a second sentence rather
                        // than a longer word.
                        described.push(' ');
                        describing = true;
                    }
                    depth += 1;
                }
                Event::End(_) if depth == 0 => break,
                Event::End(_) => {
                    depth -= 1;
                    describing &= depth > 0;
                }
                // Text arrives with its references already split out as
                // events of their own, so what is left is the characters -
                // a bare `&` among them, where a description was typed by
                // hand - and nothing here is unescaped a second time.
                Event::Text(text) if describing => {
                    described.push_str(&String::from_utf8_lossy(text.as_ref()));
                }
                Event::GeneralRef(held) if describing => {
                    let raw = format!("&{};", String::from_utf8_lossy(held.as_ref()));
                    let held = quick_xml::escape::unescape(&raw)
                        .map_err(|error| self.malformed(&error.to_string()))?;
                    described.push_str(&held);
                }
                // A CDATA section is how a description holds a `<` or an `&`
                // without escaping one, so it is content and never unescaped.
                Event::CData(text) if describing => {
                    described.push_str(&String::from_utf8_lossy(text.as_ref()));
                }
                _ => {}
            }
            buffer.clear();
        }
        Ok(Some(single_line(&described)).filter(|held| !held.is_empty()))
    }

    /// One `vocabulary-tag` as a dictionary field.
    ///
    /// A declaration this reader cannot make a field of is dropped with a
    /// warning and the vocabulary keeps the rest, which is [`Parse::dropped`]'s
    /// rule. Only the description is finer-grained than that: a tag whose
    /// wording the core will not store is still a tag, so the wording alone
    /// goes.
    fn push_tag(&mut self, element: &BytesStart<'_>, described: Option<&str>) -> Result<()> {
        let Some(name) = self.attribute(element, "name") else {
            self.dropped(
                &self.refused_in(element, "a vocabulary-tag naming its tag", "one without"),
                "the declaration is dropped",
            );
            return Ok(());
        };
        let Some(tag) = super::field::parse_tag(&name) else {
            self.dropped(
                &self.refused_in(
                    element,
                    "a decimal tag",
                    format_args!("{:?}", elide_to(&name, ERROR_TEXT_LIMIT)),
                ),
                "the declaration is dropped",
            );
            return Ok(());
        };
        let declared = self
            .attribute(element, "type")
            .unwrap_or_else(|| "string".to_owned());
        // A word neither the CBlock vocabulary nor the schema grammar reads
        // types the tag as text: the tag is still on the wire, every FIX
        // datatype is text there, and a tag dropped here would take every
        // constraint naming it out of every message that binds it.
        let dtype = cblock_type(&declared).unwrap_or_else(|| {
            self.dropped(
                &self.refused_in(
                    element,
                    format_args!(
                        "one of the CBlock types ({}) or a datatype name",
                        TYPES.join(", ")
                    ),
                    format_args!("{:?}", elide_to(&declared, ERROR_TEXT_LIMIT)),
                ),
                "the tag is typed string, which every FIX datatype is on the wire",
            );
            DataType::utf8()
        });

        // The file's own spelling kept beside the name: name resolution
        // folds ASCII case and separators, so a caller spelling it the file's
        // way still resolves and nothing is lost.
        let alt = self.attribute(element, "alt");
        // Named by the catalog name the spelling folds to - lower case, every
        // run of what no catalog name holds one `_` - so a reference can name
        // the field whatever the file wrote: `OTC Trade Flags` is
        // `otc_trade_flags`. A spelling nothing of which folds leaves the tag
        // named by its decimal, as a tag declaring no `alt` is.
        let spelling = alt.clone().unwrap_or_else(|| name.clone());
        let named = catalog_name(&spelling).map_or_else(|| name.clone(), String::from);
        let mut field = dtype.nullable_field(named.as_str());
        // The file's own spelling first: `set_metadata` replaces the whole
        // snapshot, so anything written into the `FIX:` namespace before it
        // would be replaced away.
        if let Some(alt) = alt.filter(|held| *held != named)
            && let Err(error) = field.set_metadata([("display", alt.as_str())])
        {
            self.dropped(
                &self.refused_by(
                    format_args!("tag {tag} spelling {:?}", elide_to(&alt, ERROR_TEXT_LIMIT)),
                    &error,
                ),
                "the declaration is dropped",
            );
            return Ok(());
        }
        if let Err(error) = FixFieldMut::new(&mut field)
            .set_tag(tag)
            .and_then(|()| self.stamp(&mut field))
        {
            self.dropped(
                &self.refused_by(format_args!("tag {tag}"), &error),
                "the declaration is dropped",
            );
            return Ok(());
        }
        if let Some(described) = described
            && let Err(error) = FixFieldMut::new(&mut field).set_description(described)
        {
            self.dropped(
                &self.refused_by(
                    format_args!(
                        "tag {tag} description {:?}",
                        elide_to(described, ERROR_TEXT_LIMIT)
                    ),
                    &error,
                ),
                "the tag keeps no description",
            );
        }
        self.positions.insert(tag, self.vocabulary.len());
        self.vocabulary.push(Declared {
            tag,
            position: self.position(),
            codes: Vec::new(),
            skipped: Vec::new(),
            field,
        });
        Ok(())
    }

    /// Reads every `map` into the code set of the tag it decodes.
    ///
    /// A map is named for the vocabulary tag it decodes - `ADVSIDE` decodes
    /// `AdvSide` - so the name resolves through the vocabulary this file has
    /// already read, and a map naming no tag is skipped rather than refused:
    /// a CBlock maps things that are not fields.
    ///
    /// That name is also what orients the entries, so it is resolved before
    /// the first one is read. [`Parse::decodes`] carries the rule.
    fn read_maps(&mut self) -> Result<()> {
        let mut buffer = Vec::new();
        let mut depth = 0_usize;
        loop {
            let event = self
                .reader
                .read_event_into(&mut buffer)
                .map_err(|error| self.malformed(&error.to_string()))?;
            match event {
                Event::Eof => return Err(self.unclosed("maps")),
                Event::Start(element) if is_named(&element, b"map") => {
                    let position = self.position();
                    let named = self.attribute(&element, "name");
                    let decodes = named.as_deref().and_then(|named| self.decodes(named));
                    // A map naming no tag is still read to its end: skipping
                    // the entries would leave the reader inside them.
                    let (codes, skipped) =
                        self.read_map_entries(decodes.is_some_and(|(_, fix)| fix))?;
                    // The name is what resolved the tag, so a map that
                    // decodes one has one to quote in a refusal.
                    if let Some(((at, _), named)) = decodes.zip(named.as_deref()) {
                        self.vocabulary[at].skipped.extend(skipped.into_iter().map(
                            |(key, value)| Skipped {
                                map: named.to_owned(),
                                key,
                                value,
                                position,
                            },
                        ));
                        self.attach_codes(at, &codes, named)?;
                    }
                }
                Event::Start(_) => depth += 1,
                Event::End(element) => {
                    if is_named(&element, b"maps") && depth == 0 {
                        break;
                    }
                    depth = depth.saturating_sub(1);
                }
                _ => {}
            }
            buffer.clear();
        }
        Ok(())
    }

    /// One map's entries, each oriented the way the map's name says, and the
    /// entries skipped because they spell no code, as `(key, value)`.
    ///
    /// `fix` is what [`Parse::decodes`] resolved: the FIX way round reads
    /// `key` as the wire value, the UlMessage way reads `value` as it.
    fn read_map_entries(&mut self, fix: bool) -> Result<(Vec<FixCode>, Vec<Entry>)> {
        let mut codes: Vec<FixCode> = Vec::new();
        let mut skipped = Vec::new();
        let mut buffer = Vec::new();
        let mut depth = 0_usize;
        loop {
            let event = self
                .reader
                .read_event_into(&mut buffer)
                .map_err(|error| self.malformed(&error.to_string()))?;
            match event {
                Event::Eof => return Err(self.unclosed("map")),
                Event::Empty(element) if is_named(&element, b"entry") => {
                    self.push_entry(&element, fix, &mut codes, &mut skipped);
                }
                Event::Start(element) if is_named(&element, b"entry") => {
                    self.push_entry(&element, fix, &mut codes, &mut skipped);
                    depth += 1;
                }
                Event::Start(_) => depth += 1,
                Event::End(element) => {
                    if is_named(&element, b"map") && depth == 0 {
                        break;
                    }
                    depth = depth.saturating_sub(1);
                }
                _ => {}
            }
            buffer.clear();
        }
        Ok((codes, skipped))
    }

    /// One `entry` as a code, oriented the way the map's name says.
    ///
    /// **An entry spelling nothing states no code.** A venue's map writes
    /// `none`, `null` or nothing at all where it has no code to state -
    /// `<entry key="0" value="none"/>` - so an entry whose key or value is
    /// one of those, trimmed and folded, is skipped and recorded in
    /// `skipped`, never entering the set, never claiming a name and never
    /// standing in the way of a code the dictionary already holds. An absent
    /// attribute says what an empty one says. The skipped entries are named
    /// once per code set, when the dictionary is built.
    ///
    /// Everything else is kept, because a CBlock names one wire value twice
    /// routinely - `7` is both `accountiscarriedonnoncustomersmargined` and
    /// `accountishousetraderandcrossmargined` - and a set that took the first
    /// name and dropped the second would answer to one spelling fewer than
    /// the file declared. A second name for a value already held is what a
    /// code set calls an alias, so it is added as one.
    ///
    /// A name another code already claims is the one thing that cannot be
    /// kept: two codes one spelling reaches resolve to nothing rather than
    /// to either, so the name is dropped. The value is still a fact about the
    /// wire, so a value the set does not hold yet is kept under no name - a
    /// code named after its own value - unless that value is itself another
    /// code's name. A new code is held to the rule
    /// [`FixCodes::render`](super::codes::FixCodes) refuses on as well, so
    /// one entry folding onto a code named after its own wire value costs
    /// that entry's name rather than the whole map.
    fn push_entry(
        &self,
        element: &BytesStart<'_>,
        fix: bool,
        codes: &mut Vec<FixCode>,
        skipped: &mut Vec<Entry>,
    ) {
        let key = self.attribute(element, "key").unwrap_or_default();
        let held = self.attribute(element, "value").unwrap_or_default();
        if is_sentinel(&key) || is_sentinel(&held) {
            skipped.push((key, held));
            return;
        }
        let (value, name) = if fix { (key, held) } else { (held, key) };
        let claimed = codes.iter().any(|code| claims(code, &name, &value));
        match codes.iter().position(|code| code.value() == value) {
            Some(at) if !claimed => codes[at].push_alias(name),
            Some(_) => {}
            None if !claimed => codes.push(FixCode::new(name, value)),
            // The value is a fact about the wire whatever it is called: it is
            // kept under no name where its name is another code's, unless the
            // value is itself another code's name.
            None => {
                if !codes.iter().any(|code| names_collide(code, &value, &value)) {
                    codes.push(FixCode::new(value.clone(), value));
                }
            }
        }
    }

    /// The vocabulary position a map decodes, and whether it is written the
    /// FIX way round.
    ///
    /// A CBlock writes its maps two ways: `key="0" value="day"` under a map
    /// named for the field as the file spells it, and `key="buy" value="B"`
    /// under one named the UlMessage way. Nothing in an entry separates
    /// them: both attributes are free text, and a length or a word shape is
    /// a guess that puts `B` in a name and `buy` on the wire as soon as a
    /// code set is spelled the other way round. The map's name is what does,
    /// so it decides once for the whole map.
    ///
    /// A name equal to [`spelled`] *byte for byte* is the FIX way and `key`
    /// is the wire value. Anything else is the UlMessage way and `value` is:
    /// `ADVSIDE`, `advside` and `Adv_Side` alike, because none of them is how
    /// the file spells `AdvSide`. Only resolution is forgiving - it folds
    /// case and separators like every other name in this crate, so all four
    /// reach the field.
    ///
    /// The spelling is tried before the fold, and both from the end. Two tags
    /// can fold to one name, and the one a map *spells* is
    /// the one it decodes; and where a file declares one tag twice, the last
    /// declaration is the entry the dictionary keeps.
    ///
    /// A name two *tags* answer to decodes neither, for the reason
    /// [`Parse::settle_names`] leaves them unnamed: a map is one code set and
    /// the file does not say which of the two it belongs on, so taking the
    /// last would put a venue's codes on a field at random.
    ///
    /// The rule reads `alt` as the FIX spelling, which is what the corpus
    /// writes. A file spelling it the UlMessage way instead - `alt="SIDE"`
    /// under `<map name="SIDE">` - is read the FIX way and inverted, and
    /// nothing in the document separates that from a map that means it.
    fn decodes(&self, named: &str) -> Option<(usize, bool)> {
        let named = named.trim();
        if let Some(at) = self.decoded(|held| spelled(&held.field) == named) {
            return Some((at, true));
        }
        // The name a field is filed under folds its punctuation away, so a
        // map spelled with it reaches the field by the same fold.
        let folded = catalog_name(named);
        let at = self.decoded(|held| {
            crate::implementer::folds_equal(held.field.name(), named)
                || folded.as_deref().is_some_and(|folded| {
                    crate::implementer::folds_equal(held.field.name(), folded)
                })
        })?;
        Some((at, false))
    }

    /// The one declaration a probe reaches, taken from the end, or none where
    /// it reaches two tags.
    fn decoded(&self, probe: impl Fn(&Declared) -> bool) -> Option<usize> {
        let at = self.vocabulary.iter().rposition(&probe)?;
        let tag = self.vocabulary[at].tag;
        self.vocabulary[..at]
            .iter()
            .all(|held| held.tag == tag || !probe(held))
            .then_some(at)
    }

    /// Holds one code set for the vocabulary field its map decodes.
    ///
    /// An empty set is dropped rather than held: a map declaring no entries is
    /// not the file saying the field has no codes. The members are filed under
    /// a name when the dictionary is built, because that is where a name can
    /// be given and a refusal can name the entry it came from.
    fn attach_codes(&mut self, at: usize, codes: &[FixCode], named: &str) -> Result<()> {
        if codes.is_empty() {
            return Ok(());
        }
        let tag = self.vocabulary[at].tag;
        if let Err(error) = FixCodes::render(codes) {
            self.dropped(
                &self.refused_by(
                    format_args!(
                        "map {:?} on tag {tag}, code set {}",
                        elide_to(named, ERROR_TEXT_LIMIT),
                        FixRegistry::derived_codeset_name(&self.vocabulary[at].field)
                    ),
                    &error,
                ),
                "the tag keeps no code set",
            );
            return Ok(());
        }
        self.vocabulary[at].codes = codes.to_vec();
        Ok(())
    }

    /// Reads every `message-type` the file lists.
    ///
    /// `value` is what the bridge calls the type - `7`, `P Report Ack` - and
    /// `description` is the wording beside it. Neither `supported` nor
    /// `rejection` is read: a type this dialect refuses is still a type it
    /// names, and a dictionary that dropped it would answer nothing for the
    /// traffic someone is reading a rejection over.
    fn read_message_types(&mut self) -> Result<()> {
        let mut buffer = Vec::new();
        let mut depth = 0_usize;
        loop {
            let event = self
                .reader
                .read_event_into(&mut buffer)
                .map_err(|error| self.malformed(&error.to_string()))?;
            match event {
                Event::Eof => return Err(self.unclosed("message-types")),
                Event::Start(element) | Event::Empty(element)
                    if is_named(&element, b"message-type") =>
                {
                    let spelling = self.attribute(&element, "value");
                    let described = self.attribute(&element, "description");
                    if let Some(spelling) = spelling.filter(|held| !held.trim().is_empty()) {
                        self.push_msgtype(&spelling, described.as_deref());
                    }
                }
                Event::Start(_) => depth += 1,
                Event::End(element) => {
                    if is_named(&element, b"message-types") && depth == 0 {
                        break;
                    }
                    depth = depth.saturating_sub(1);
                }
                _ => {}
            }
            buffer.clear();
        }
        Ok(())
    }

    /// Reads one mapping table for the spellings it gives a message type.
    ///
    /// `<entry key="allocationreportack" value="P Report Ack" />` is one
    /// message type under two spellings: the value is the type as the rest of
    /// the file spells it, and the key is the name UlMessage uses. The pair is
    /// an alias and never a second type, and never a type at all: the value
    /// has to name a wire type the `message-types` listing above it already
    /// declared, and an entry naming anything else is a mapping to nothing
    /// and is dropped with a warning. A table is where a file spells its
    /// types the way UlMessage does, not where it declares them, so taking
    /// one at its word would hang `allocation` on whatever `J` a typo
    /// produced and answer it forever after.
    ///
    /// Checked by wire value rather than by the whole spelling, because that
    /// is what one type is: a listing that declares `J Report` alone has
    /// declared the type an entry spells `J`, and the two are one code.
    fn read_msgtype_mappings(&mut self, name: &[u8]) -> Result<()> {
        let mut buffer = Vec::new();
        let mut depth = 0_usize;
        loop {
            let event = self
                .reader
                .read_event_into(&mut buffer)
                .map_err(|error| self.malformed(&error.to_string()))?;
            match event {
                Event::Eof => return Err(self.unclosed(&String::from_utf8_lossy(name))),
                Event::Start(element) | Event::Empty(element) if is_named(&element, b"entry") => {
                    // A side spelling nothing names nothing, as in a map.
                    let key = self
                        .attribute(&element, "key")
                        .filter(|held| !is_sentinel(held));
                    let spelling = self.attribute(&element, "value");
                    if let Some(spelling) = spelling.filter(|held| !is_sentinel(held)) {
                        let spelled = spelling.trim();
                        let Some(at) = self
                            .values
                            .get(super::msgtype::wire_value(spelled))
                            .copied()
                        else {
                            self.dropped(
                                &self.refused_in(
                                    &element,
                                    "a message type this file's listing declares",
                                    format_args!("{:?}", elide_to(spelled, ERROR_TEXT_LIMIT)),
                                ),
                                "the mapping is dropped",
                            );
                            buffer.clear();
                            continue;
                        };
                        // The entry's own spelling too: a listing that wrote
                        // `J Report` and a table that writes `J` are one type
                        // under two names, and both are names it answers to.
                        self.alias_msgtype(at, spelled);
                        if let Some(key) = key {
                            self.alias_msgtype(at, &key);
                        }
                    }
                }
                Event::Start(_) => depth += 1,
                Event::End(element) => {
                    if element.local().as_ref() == name && depth == 0 {
                        break;
                    }
                    depth = depth.saturating_sub(1);
                }
                _ => {}
            }
            buffer.clear();
        }
        Ok(())
    }

    /// One message type as a code of tag 35, answering where it sits.
    ///
    /// **The wire value is the code's name and the qualified spelling is only
    /// ever a spelling of it.** [`wire_value`](super::msgtype::wire_value)
    /// splits the two, so `6 Inbound` declares tag 35 `6` and answers to `6
    /// Inbound`; it never adds a code called `6 Inbound`, and where the
    /// dictionary this folds into already names `6`, that name is the one the
    /// merged set keeps. Two declarations of one wire type - `AR Inbound` and
    /// `AR Outbound`, `c SDR` and `c SLR` - are therefore one code answering
    /// to every spelling the file gave it, and the first wording it gave is
    /// the description it keeps.
    fn push_msgtype(&mut self, spelling: &str, described: Option<&str>) -> Option<usize> {
        let spelling = spelling.trim();
        let value = super::msgtype::wire_value(spelling);
        let described = described.map(single_line).filter(|held| !held.is_empty());
        if let Some(at) = self.values.get(value).copied() {
            if self.msgtypes[at].description().is_none()
                && let Some(described) = described
            {
                self.msgtypes[at] = self.msgtypes[at].clone().with_description(described);
            }
            self.alias_msgtype(at, spelling);
            return Some(at);
        }
        // A wire value is never contended by the fold. `self.values` keys it
        // exactly, which is the only reading tag 35 has: `b` and `B` are two
        // messages and so are `c` and `C`, so a file declaring both declares
        // two types rather than one it spelled twice. The fold owns the
        // spellings below and stops at the values.
        let mut code = FixCode::new(value, value);
        if let Some(described) = described {
            code = code.with_description(described);
        }
        self.msgtypes.push(code);
        let at = self.msgtypes.len() - 1;
        self.values.insert(SmolStr::new(value), at);
        // The whole spelling, and only as a spelling: a qualifier says which
        // grammar the file bound, never what the type is called. A spelling
        // that is the bare wire value claims nothing in the folded namespace,
        // because `self.values` already holds it under the one reading it has.
        self.alias_msgtype(at, spelling);
        Some(at)
    }

    /// Adds one more spelling that reaches the message type at `at`.
    ///
    /// A spelling another code already answers to is dropped rather than
    /// added, because two codes one spelling reaches resolve to neither -
    /// the rule a map's entries are read under.
    ///
    /// The bare wire value is the exception, and it is not a spelling: a code
    /// answers its own value exactly, `self.values` is where that is keyed,
    /// and claiming its fold here would let `B` hold the name `b` answers to.
    /// It is still pushed as an alias where the code does not already carry
    /// it, so nothing the file said is lost.
    fn alias_msgtype(&mut self, at: usize, spelling: &str) {
        let spelling = spelling.trim();
        if spelling.is_empty() {
            return;
        }
        if spelling == self.msgtypes[at].value() {
            return;
        }
        let key = crate::implementer::normalized(spelling);
        if self.spellings.contains_key(&key) {
            return;
        }
        self.msgtypes[at].push_alias(spelling);
        self.spellings.insert(key, at);
    }

    /// Puts the message types the file declared on its own tag 35.
    ///
    /// A CBlock states its types beside its vocabulary rather than inside it,
    /// so this is the one write that crosses the two. A file declaring no tag
    /// 35 has nowhere to put them and keeps them out of the dictionary rather
    /// than inventing the field: a message type is a code of tag 35 and never
    /// a field of its own.
    fn attach_msgtypes(&mut self) -> Result<()> {
        if self.msgtypes.is_empty() {
            return Ok(());
        }
        let Some(at) = self.positions.get(&MSGTYPE_TAG_NAME.0).copied() else {
            return Ok(());
        };
        let mut codes = std::mem::take(&mut self.msgtypes);
        // What the file already said about tag 35 - a `map` naming it - leads,
        // because a map states the wire value and a message type is named by
        // the bridge; a spelling either already holds is not added twice.
        let held = std::mem::take(&mut self.vocabulary[at].codes);
        codes.retain(|code| {
            !held
                .iter()
                .any(|kept| kept.value() == code.value() || kept.is_spelled(code.name()))
        });
        codes.splice(0..0, held);
        let position = self.vocabulary[at].position;
        if let Err(error) = FixCodes::render(&codes) {
            self.dropped(
                &self.refusal_at(
                    position,
                    format_smolstr!(
                        "the message types tag {} carries: {error}",
                        MSGTYPE_TAG_NAME.0
                    ),
                ),
                "the message types are not filed as that tag's code set",
            );
            return Ok(());
        }
        self.vocabulary[at].codes = codes;
        Ok(())
    }

    /// Reads every `tag-normalization` a `normalization-binding` states, for
    /// the names it spells its tags with.
    ///
    /// A binding nests: a repeating group's members sit inside a
    /// `normalization` of their own, and that one sits beside its siblings
    /// rather than under them. Depth is counted rather than assumed, and a
    /// `tag-normalization` is read wherever it appears, because a name a
    /// group's member is spelled with is that member's name.
    ///
    /// Everything else the binding holds is read past. `type` is not read at
    /// either level - the corpus writes a message type there and writes a
    /// direction there - because a name is a name whichever way the file was
    /// binding it, and reading the attribute would only make this parser
    /// choose between two readings it has no evaluator to check.
    fn read_normalization_binding(&mut self) -> Result<()> {
        let mut buffer = Vec::new();
        let mut depth = 0_usize;
        loop {
            let event = self
                .reader
                .read_event_into(&mut buffer)
                .map_err(|error| self.malformed(&error.to_string()))?;
            match event {
                Event::Eof => return Err(self.unclosed("normalization-binding")),
                Event::Start(element) if is_named(&element, b"tag-normalization") => {
                    // Owned because the mapping is read before the attribute
                    // is, and reading it moves the reader off this element.
                    let element = element.into_owned();
                    let mapped = self.read_mapping()?;
                    self.push_spelling(&element, mapped)?;
                }
                // One that closes on itself maps nothing, so it names nothing.
                Event::Empty(_) => {}
                Event::Start(_) => depth += 1,
                Event::End(element) => {
                    if is_named(&element, b"normalization-binding") && depth == 0 {
                        break;
                    }
                    depth = depth.saturating_sub(1);
                }
                _ => {}
            }
            buffer.clear();
        }
        Ok(())
    }

    /// The tag one `tag-normalization` states its name for, where it states
    /// one and computes nothing.
    ///
    /// A `mapping-condition` is what makes a mapping conditional, and a name
    /// that means a tag only when another tag holds a particular value is not
    /// another spelling of it: `LEGISINCODE` is `$602` under `$603 = "4"` and
    /// `LEGEXCHANGECODE` is the same `$602` under `$603 = "8"`, so taking
    /// either as a name for tag 602 would give that tag two names it answers
    /// to unconditionally and neither of them what the file said. A
    /// conditional mapping therefore names nothing here.
    ///
    /// Two expressions name nothing either. One is a mapping and several are
    /// a construction, and this layer holds no evaluator to say what the
    /// construction would come to.
    fn read_mapping(&mut self) -> Result<Option<i32>> {
        let mut buffer = Vec::new();
        let mut depth = 0_usize;
        // The two facts a mapping is read for. `mapping` counts the
        // expressions under `mapping-expression` so a second one is a
        // construction rather than a replacement of the first.
        let mut conditional = false;
        let mut mapping = 0_usize;
        let mut mapped = None;
        // Where in the element the reader is: only a `mapping-expression`'s
        // own `expression` maps, and a `mapping-condition` carries the same
        // element name for the opposite purpose.
        let mut mapping_at = None;
        loop {
            let event = self
                .reader
                .read_event_into(&mut buffer)
                .map_err(|error| self.malformed(&error.to_string()))?;
            let element = match &event {
                Event::Start(element) | Event::Empty(element) => Some(element),
                _ => None,
            };
            if let Some(element) = element {
                if is_named(element, b"mapping-condition") {
                    // An empty one states no condition, exactly as an empty
                    // `condition-expression` does.
                    conditional |= matches!(event, Event::Start(_));
                } else if is_named(element, b"mapping-expression")
                    && mapping_at.is_none()
                    && matches!(event, Event::Start(_))
                {
                    // One that closes on itself holds no expression, so there
                    // is no level under it for one to be read at.
                    mapping_at = Some(depth);
                } else if is_named(element, b"expression")
                    && mapping_at.is_some_and(|at| at + 1 == depth)
                {
                    mapping += 1;
                    if mapping == 1 {
                        mapped = self
                            .attribute(element, "value")
                            .as_deref()
                            .and_then(referenced);
                    }
                }
            }
            match event {
                Event::Eof => return Err(self.unclosed("tag-normalization")),
                Event::Start(_) => depth += 1,
                Event::End(_) if depth == 0 => break,
                Event::End(_) => {
                    depth -= 1;
                    if mapping_at == Some(depth) {
                        mapping_at = None;
                    }
                }
                _ => {}
            }
            buffer.clear();
        }
        Ok(mapped.filter(|_| !conditional && mapping == 1))
    }

    /// One `tag-normalization` as a name its tag is spelled with.
    ///
    /// An absent or empty `tag-name` says nothing, exactly as an absent map
    /// entry attribute does, and a mapping that names no single tag has no
    /// tag to spell - both drop the element rather than refuse the file,
    /// because a binding this parser otherwise reads past is not a place to
    /// refuse a whole dictionary from.
    fn push_spelling(&mut self, element: &BytesStart<'_>, tag: Option<i32>) -> Result<()> {
        let name = self
            .attribute(element, "tag-name")
            .map(|held| held.trim().to_owned())
            .filter(|held| !held.is_empty());
        let (Some(tag), Some(name)) = (tag, name) else {
            return Ok(());
        };
        self.named.push(Spelled {
            tag,
            name,
            position: self.position(),
        });
        Ok(())
    }

    /// Attaches every name a normalization spelled to the tag it spelled it
    /// for.
    ///
    /// The vocabulary owns what a tag is called and this only adds spellings
    /// beside it, so every name arrives as an alias and none replaces a name.
    /// Which is the whole of what it is worth reading for: a `vocabulary-tag`
    /// declaring no `alt` is named by its own decimal tag, and the
    /// normalization is then the only place the file says what that tag is
    /// called.
    ///
    /// Four names are dropped rather than stored, and none of them is a
    /// defect in the file:
    ///
    /// - one the tag already answers to. A name resolves through ASCII case,
    ///   so `LEGSECURITYID` already reaches a tag the vocabulary spelled
    ///   `LegSecurityID` and an alias for it would store a spelling that
    ///   resolves without one. Most of a real binding is this case.
    /// - one the tag already carries as an alias, because a file spells a tag
    ///   in as many normalizations as it maps the tag in.
    /// - one another tag already answers to, canonically or as an alias.
    ///   Two fields one spelling reaches resolve to neither,
    ///   so the second claim is dropped exactly as a map entry's second claim
    ///   on one name is - and a dictionary that refuses the whole file over
    ///   it would refuse a document nothing is wrong with.
    /// - one carrying the separator a stored alias list is rendered with,
    ///   which is the one spelling the core cannot hold.
    ///
    /// The two tests fold differently, and the dictionary is why. What a tag
    /// *already answers to* is decided by ASCII case, because that is what
    /// resolution rechecks a name with - so `LEG_SECURITY_ID` is a spelling
    /// tag 602 does not answer to and is worth storing even where
    /// `LEGSECURITYID` is not. What is *already claimed* is decided by the
    /// whole fold, separators included, because that is what the alias index
    /// keys on: two spellings one key reaches are one claim however they are
    /// punctuated, and the second would be refused rather than shadowed.
    fn attach_names(&mut self) -> Result<()> {
        if self.named.is_empty() {
            return Ok(());
        }
        // Indexed rather than scanned, for the reason the vocabulary is: a
        // real binding spells thousands of names against a vocabulary of
        // thousands, and a scan per name is the parse squared. Keyed by what
        // the dictionary indexes a name under, and holding the tag beside the
        // entry: a fold two *tags* already answer to is claimed by neither, so
        // a binding cannot spell a contended name back onto one of them and
        // undo what [`Parse::settle_names`] settled. A tag its file declares
        // twice claims once, because that is one tag.
        let mut claimed: std::collections::HashMap<u64, Option<(i32, usize)>> =
            std::collections::HashMap::new();
        for (at, held) in self.vocabulary.iter().enumerate() {
            let field = &held.field;
            let spellings = [field.name(), spelled(field)]
                .into_iter()
                .chain(FixField::new(field).names());
            for spelling in spellings {
                claimed
                    .entry(super::registry::name_key(spelling))
                    .and_modify(|prior| {
                        if prior.is_some_and(|(claimant, _)| claimant != held.tag) {
                            *prior = None;
                        }
                    })
                    .or_insert(Some((held.tag, at)));
            }
        }
        for held in std::mem::take(&mut self.named) {
            let Spelled {
                tag,
                name,
                position,
            } = held;
            let Some(at) = self.positions.get(&tag).copied() else {
                continue;
            };
            if name.contains(',') {
                continue;
            }
            let key = super::registry::name_key(&name);
            match claimed.get(&key) {
                Some(Some((claimant, _))) if *claimant != tag => continue,
                Some(None) => continue,
                _ => {}
            }
            let field = &mut self.vocabulary[at].field;
            if name.eq_ignore_ascii_case(field.name()) || name.eq_ignore_ascii_case(spelled(field))
            {
                continue;
            }
            let mut aliases: Vec<SmolStr> =
                FixField::new(field).names().map(SmolStr::new).collect();
            if aliases.iter().any(|held| name.eq_ignore_ascii_case(held)) {
                continue;
            }
            aliases.push(SmolStr::new(&name));
            if let Err(error) = FixFieldMut::new(field).set_names(&aliases) {
                self.dropped(
                    &self.refusal_at(
                        position,
                        format_smolstr!(
                            "tag {tag} spelled {:?}: {error}",
                            elide_to(&name, ERROR_TEXT_LIMIT)
                        ),
                    ),
                    "the spelling is dropped",
                );
                continue;
            }
            claimed.insert(key, Some((tag, at)));
        }
        Ok(())
    }

    /// Reads one `grammar-binding` into one message root.
    ///
    /// The root is a non-null struct named by the binding's `type` verbatim,
    /// spaces and case included, so `7` and `P Report Ack` are both legal
    /// names. This is the one place the lower-case law does not reach: a
    /// MsgType is case-bearing, `A` is Logon and `a` is QuoteStatusRequest, so
    /// folding a root name would merge two messages.
    fn read_binding(&mut self, element: &BytesStart<'_>) -> Result<()> {
        let declared = self.attribute(element, "type");
        // A file binds types its own listing sometimes omits, and a bound type
        // is one this dialect carries: the binding declares it too. The
        // unknown root is this crate's word for a binding that named nothing,
        // so it is not one of them.
        if let Some(declared) = declared.as_deref().filter(|held| !held.trim().is_empty()) {
            self.push_msgtype(declared, None);
        }
        let msgtype = declared.unwrap_or_else(|| super::build::UNKNOWN_MSGTYPE.to_owned());
        let at = self.position();
        let mut buffer = Vec::new();
        loop {
            let event = self
                .reader
                .read_event_into(&mut buffer)
                .map_err(|error| self.malformed(&error.to_string()))?;
            match event {
                Event::Eof => return Err(self.unclosed("grammar-binding")),
                Event::Start(element) if is_named(&element, b"grammar") => {
                    let children = self.read_grammar(1, &msgtype)?;
                    match StructType::from_fields(children).map(DataType::from) {
                        Ok(dtype) => self.roots.push((dtype.required_field(msgtype.clone()), at)),
                        // A root its own children will not make a struct of
                        // is one message dropped, not one file: the binding
                        // still declared the type, and every other message
                        // the file binds is untouched.
                        Err(error) => self.dropped(
                            &self.refused_by(
                                format_args!("message {:?}", elide_to(&msgtype, ERROR_TEXT_LIMIT)),
                                &error,
                            ),
                            "the message is dropped",
                        ),
                    }
                }
                // An empty root grammar is an empty non-null struct rather
                // than a failure: the file bound the message and said it holds
                // nothing, which is a statement and not a defect.
                Event::Empty(element) if is_named(&element, b"grammar") => {
                    let root = DataType::from(StructType::from_fields([])?)
                        .required_field(msgtype.clone());
                    self.roots.push((root, at));
                }
                Event::End(element) if is_named(&element, b"grammar-binding") => break,
                _ => {}
            }
            buffer.clear();
        }
        Ok(())
    }

    /// The children of one `grammar`, in document order.
    ///
    /// One recursive function, because a nested grammar is a sibling as often
    /// as a child: a grammar routinely holds several, separated by plain
    /// constraints, so a two-level special case would read the file wrongly.
    fn read_grammar(&mut self, depth: usize, msgtype: &str) -> Result<Vec<Field>> {
        if depth > MAX_DEPTH {
            self.dropped(
                &self.refused(
                    format_args!("a grammar nested at most {MAX_DEPTH} deep"),
                    format_args!(
                        "a deeper one in message {:?}",
                        elide_to(msgtype, ERROR_TEXT_LIMIT)
                    ),
                ),
                "the grammar is dropped",
            );
            // Read to its own end before answering, or the reader would carry
            // on inside a grammar nobody is reading.
            self.skip(b"grammar")?;
            return Ok(Vec::new());
        }
        let mut children: Vec<Field> = Vec::new();
        let mut buffer = Vec::new();
        loop {
            let event = self
                .reader
                .read_event_into(&mut buffer)
                .map_err(|error| self.malformed(&error.to_string()))?;
            match event {
                Event::Eof => return Err(self.unclosed("grammar")),
                Event::Start(element) if is_named(&element, b"tag-constraint") => {
                    if let Some(field) = self.read_constraint(&element, false, msgtype)? {
                        push_member(&mut children, field);
                    }
                }
                Event::Empty(element) if is_named(&element, b"tag-constraint") => {
                    if let Some(field) = self.read_constraint(&element, true, msgtype)? {
                        push_member(&mut children, field);
                    }
                }
                Event::Start(element) if is_named(&element, b"grammar") => {
                    let declared = self.attribute(&element, "rg-name");
                    let nested = self.read_grammar(depth + 1, msgtype)?;
                    if let Some(group) = self.grouped(nested, msgtype, declared.as_deref()) {
                        push_member(&mut children, group);
                    }
                }
                // A nested grammar with no children has no counter to name it,
                // so the group goes and the message keeps the rest.
                Event::Empty(element) if is_named(&element, b"grammar") => {
                    self.dropped(&self.counterless(msgtype), GROUP_DROPPED);
                }
                Event::End(element) if is_named(&element, b"grammar") => break,
                _ => {}
            }
            buffer.clear();
        }
        Ok(children)
    }

    /// One nested grammar's children as a repeating-group field.
    ///
    /// The grammar's opening constraint is its counter: it names the group
    /// and its tag frames the group on the wire as the group's `FIX:counter`,
    /// never a member beside it - the serie's length is the count. Its item
    /// holds the members that follow.
    fn grouped(
        &mut self,
        mut children: Vec<Field>,
        msgtype: &str,
        declared: Option<&str>,
    ) -> Option<Field> {
        if children.is_empty() {
            self.dropped(&self.counterless(msgtype), GROUP_DROPPED);
            return None;
        }
        let counter = children.remove(0);
        // A group whose first child is another grammar has no counter to name
        // it, so it is dropped while the parent keeps the rest.
        if matches!(
            counter.dtype(),
            DataType::Serie(_) | DataType::LargeSerie(_)
        ) {
            self.dropped(
                &self.refused(
                    "a nested grammar opening with its counter",
                    format_args!(
                        "one opening with the group {:?} in message {:?}",
                        elide_to(counter.name(), ERROR_TEXT_LIMIT),
                        elide_to(msgtype, ERROR_TEXT_LIMIT)
                    ),
                ),
                "the group is dropped and the message keeps the rest",
            );
            return None;
        }
        let named = SmolStr::new(counter.name());
        let dropping = |parse: &Self, error: &Error| {
            parse.dropped(
                &parse.refused_by(
                    format_args!(
                        "group {:?} in message {:?}",
                        elide_to(&named, ERROR_TEXT_LIMIT),
                        elide_to(msgtype, ERROR_TEXT_LIMIT)
                    ),
                    error,
                ),
                GROUP_DROPPED,
            );
        };
        let tag = match FixField::new(&counter).tag() {
            Ok(Some(tag)) => tag,
            Ok(None) => {
                self.dropped(&self.counterless(msgtype), GROUP_DROPPED);
                return None;
            }
            Err(error) => {
                dropping(self, &error);
                return None;
            }
        };
        if let Some(position) = self.positions.get(&tag).copied()
            && let Err(error) = self.vocabulary[position].field.set_dtype(DataType::Int32)
        {
            dropping(self, &error);
            return None;
        }
        let (group_name, group_display, occurrence_name, occurrence_display) =
            super::component::group_names(&counter, declared);
        // Named as a catalog files a name, whatever the `rg-name` or the
        // counter's spelling holds - `(BloombergLegs)` is `bloomberglegs` -
        // the spelling kept as the display; one nothing of which folds is
        // named after the counter, whose name already is a catalog name.
        let mut name = catalog_name(&group_name)
            .map_or_else(|| format!("{}grp", counter.name()), String::from);
        let mut display = group_display.trim().to_owned();
        if self
            .vocabulary
            .iter()
            .any(|field| crate::implementer::folds_equal(field.field.name(), &name))
        {
            name.push_str("grp");
            if !display.ends_with("Grp") {
                display.push_str("Grp");
            }
        }
        let mut entry = catalog_name(&occurrence_name)
            .map_or_else(|| format!("{}component", counter.name()), String::from);
        let mut entry_display = occurrence_display.trim().to_owned();
        if self
            .vocabulary
            .iter()
            .any(|field| crate::implementer::folds_equal(field.field.name(), &entry))
        {
            entry.push_str("component");
            if !entry_display.ends_with("Component") {
                entry_display.push_str("Component");
            }
        }
        let built = || -> Result<Field> {
            let mut item =
                DataType::from(StructType::from_fields(children)?).required_field(entry.clone());
            item.set_display(&entry_display)?;
            self.stamp(&mut item)?;
            let mut group = DataType::serie(item).nullable_field(name);
            group.set_display(&display)?;
            group.set_nullable(counter.is_nullable());
            self.stamp(&mut group)?;
            FixFieldMut::new(&mut group).set_counter(tag)?;
            FixFieldMut::new(&mut group).set_component(&entry)?;
            Ok(group)
        };
        match built() {
            Ok(held) => Some(held),
            Err(error) => {
                dropping(self, &error);
                None
            }
        }
    }

    /// One `tag-constraint` as a leaf field, where the file bound one.
    ///
    /// It resolves its tag in this file's own vocabulary, clones that entry
    /// and overrides only the clone's nullability - so the dictionary stays
    /// immutable and one entry serves every message that binds it.
    ///
    /// A constraint that names no tag, names something that is not one, or
    /// names one this file's own vocabulary never declared is dropped with a
    /// warning and the message keeps its other children. Its validity
    /// children are still read to their end, because the reader has to leave
    /// the element whether or not anything came of it.
    fn read_constraint(
        &mut self,
        element: &BytesStart<'_>,
        closed: bool,
        msgtype: &str,
    ) -> Result<Option<Field>> {
        let Some(name) = self.attribute(element, "name") else {
            self.dropped(
                &self.refused_in(
                    element,
                    "a tag-constraint naming its tag",
                    format_args!(
                        "one without in message {:?}",
                        elide_to(msgtype, ERROR_TEXT_LIMIT)
                    ),
                ),
                "the constraint is dropped and the message keeps its other members",
            );
            self.check_validity(closed)?;
            return Ok(None);
        };
        let Some(tag) = super::field::parse_tag(&name) else {
            self.dropped(
                &self.refused_in(
                    element,
                    "a decimal tag",
                    format_args!(
                        "{:?} in message {:?}",
                        elide_to(&name, ERROR_TEXT_LIMIT),
                        elide_to(msgtype, ERROR_TEXT_LIMIT)
                    ),
                ),
                "the constraint is dropped and the message keeps its other members",
            );
            self.check_validity(closed)?;
            return Ok(None);
        };
        let held = match self
            .positions
            .get(&tag)
            .and_then(|at| self.vocabulary.get(*at))
            .map(|held| held.field.clone())
        {
            Some(held) => held,
            // A constraint on a tag the vocabulary never declared is still
            // the file saying the tag is on the wire in this message, and
            // every FIX datatype is text there: the constraint declares the
            // tag as text, named by nothing but its digits, so the message
            // keeps the member and a file that does name the tag names this
            // field when the two fold. A stale template line costs one text
            // field the warning names; a dropped member cost the message its
            // schema for that tag.
            None => {
                self.dropped(
                    &self.refused_in(
                        element,
                        "a tag this file's vocabulary declares",
                        format_args!(
                            "tag {tag} in message {:?}",
                            elide_to(msgtype, ERROR_TEXT_LIMIT)
                        ),
                    ),
                    "the constraint declares the tag as text, named by its digits",
                );
                match self.declared_by_constraint(tag) {
                    Some(held) => held,
                    None => {
                        self.check_validity(closed)?;
                        return Ok(None);
                    }
                }
            }
        };
        // `required` is usually a flag but is sometimes a condition such as
        // `$59 = '6' and empty($126)`. An expression is read as not-required
        // rather than as a failure: this layer holds no evaluator, and a
        // conditionally required field is one that may be absent.
        let required = self
            .attribute(element, "required")
            .is_some_and(|held| crate::implementer::bool_from_text(&held) == Some(true));
        let mut field = held;
        field.set_nullable(!required);
        self.check_validity(closed)?;
        Ok(Some(field))
    }

    /// Declares the tag a constraint names and the vocabulary does not, as
    /// the text field the constraint alone says it is.
    ///
    /// Entered into the vocabulary so that every later constraint on the tag
    /// reads the one field, the dictionary registers it, and
    /// [`Parse::settle_names`] names it by its normalization where the file
    /// spells one. `None` where the core will not hold even that, which is
    /// warned and dropped.
    fn declared_by_constraint(&mut self, tag: i32) -> Option<Field> {
        let mut field = DataType::utf8().nullable_field(format_smolstr!("{tag}"));
        if let Err(error) = FixFieldMut::new(&mut field)
            .set_tag(tag)
            .and_then(|()| self.stamp(&mut field))
        {
            self.dropped(
                &self.refused_by(format_args!("tag {tag}"), &error),
                "the constraint is dropped and the message keeps its other members",
            );
            return None;
        }
        self.positions.insert(tag, self.vocabulary.len());
        self.vocabulary.push(Declared {
            tag,
            position: self.position(),
            codes: Vec::new(),
            skipped: Vec::new(),
            field: field.clone(),
        });
        Some(field)
    }

    /// Consumes a constraint's validity children, refusing an unknown domain.
    ///
    /// All of it is dropped. It is read rather than skipped so that a `domain`
    /// outside the two is a refusal instead of a silence, and so the losses
    /// this module lists can be exact.
    fn check_validity(&mut self, closed: bool) -> Result<()> {
        if closed {
            return Ok(());
        }
        let mut buffer = Vec::new();
        let mut depth = 0_usize;
        loop {
            let event = self
                .reader
                .read_event_into(&mut buffer)
                .map_err(|error| self.malformed(&error.to_string()))?;
            match event {
                Event::Eof => return Err(self.unclosed("tag-constraint")),
                Event::Start(held) => {
                    self.check_domain(&held)?;
                    depth += 1;
                }
                Event::Empty(held) => self.check_domain(&held)?,
                Event::End(held) => {
                    if is_named(&held, b"tag-constraint") && depth == 0 {
                        break;
                    }
                    depth = depth.saturating_sub(1);
                }
                _ => {}
            }
            buffer.clear();
        }
        Ok(())
    }

    /// Refuses a validity element declaring a domain outside the two.
    fn check_domain(&self, element: &BytesStart<'_>) -> Result<()> {
        let Some(domain) = self.attribute(element, "domain") else {
            return Ok(());
        };
        if DOMAINS.contains(&domain.as_str()) {
            return Ok(());
        }
        // Every validity element is dropped anyway; the domain is read so
        // that one outside the two is a warning instead of a silence.
        self.dropped(
            &self.refused_in(
                element,
                format_args!("a domain of {} or {}", DOMAINS[0], DOMAINS[1]),
                format_args!("{:?}", elide_to(&domain, ERROR_TEXT_LIMIT)),
            ),
            "the validity is read past, as every validity is",
        );
        Ok(())
    }

    /// Skips one element and everything under it, by counting depth.
    fn skip(&mut self, name: &[u8]) -> Result<()> {
        let mut buffer = Vec::new();
        let mut depth = 0_usize;
        loop {
            let event = self
                .reader
                .read_event_into(&mut buffer)
                .map_err(|error| self.malformed(&error.to_string()))?;
            match event {
                Event::Eof => {
                    return Err(self.unclosed(&String::from_utf8_lossy(name)));
                }
                Event::Start(_) => depth += 1,
                Event::End(held) => {
                    if depth == 0 && held.local_name().as_ref() == name {
                        break;
                    }
                    depth = depth.saturating_sub(1);
                }
                _ => {}
            }
            buffer.clear();
        }
        Ok(())
    }

    /// One attribute's unescaped value.
    ///
    /// Attributes are entity-escaped - a `required` condition routinely is -
    /// so nothing may be tested before it is unescaped, and the unescaping is
    /// the XML reader's own rather than a hand-written entity table.
    ///
    /// An attribute is one attribute, and the element still says everything
    /// its other attributes say: one the reader cannot split into a key and
    /// a quoted value - `activated` alone, a value left unquoted - is passed
    /// over with a warning, and one whose value will not unescape - `S&P
    /// 500`, an HTML entity - keeps the value as the file spelled it, named
    /// the same way, because the spelling is what the file said and a refusal
    /// would say nothing. An attribute a hand-edited element states twice is
    /// read once, the first statement standing.
    fn attribute(&self, element: &BytesStart<'_>, name: &str) -> Option<String> {
        for attribute in element.attributes().with_checks(false) {
            let attribute = match attribute {
                Ok(attribute) => attribute,
                Err(error) => {
                    self.dropped(
                        &self.refused_in(element, "a well-formed attribute", error),
                        "the attribute is dropped and the element keeps the rest",
                    );
                    continue;
                }
            };
            if attribute.key.local_name().as_ref() != name.as_bytes() {
                continue;
            }
            // Every CBlock declares `<?xml version="1.0"?>`, and the
            // normalization differs only for 1.1's extra control-character
            // entities, which none of the corpus carries.
            let held = match attribute.normalized_value(quick_xml::XmlVersion::Explicit1_0) {
                Ok(held) => held.into_owned(),
                Err(error) => {
                    self.dropped(
                        &self.refused_in(
                            element,
                            format_args!("an escaped {name} attribute"),
                            format_args!("{error}"),
                        ),
                        "the value is kept as the file spelled it",
                    );
                    String::from_utf8_lossy(&attribute.value).into_owned()
                }
            };
            return Some(held);
        }
        None
    }

    /// The byte the reader has reached, which every refusal is located at.
    fn position(&self) -> usize {
        self.reader.buffer_position() as usize
    }

    /// The document text just before that position, bounded.
    ///
    /// What a malformed document has instead of an element: the reader stopped
    /// before it had one, so a refusal quotes what it was reading. Lossy,
    /// because those are the bytes a refusal exists to show.
    fn reading(&self) -> String {
        let end = self.position().min(self.bytes.len());
        let start = end.saturating_sub(ERROR_TEXT_LIMIT);
        String::from_utf8_lossy(&self.bytes[start..end]).into_owned()
    }

    /// A refusal over a document the XML reader itself could not read.
    ///
    /// The reader's own sentence quotes the document too - an unmatched end
    /// tag names both spellings, an unrecognized entity names it - so it
    /// crosses the same budget every other span does.
    fn malformed(&self, reason: &str) -> Error {
        let reading = self.reading();
        self.refused(
            "a well-formed CBlock",
            format_args!(
                "{}, reading {:?}",
                elide_to(reason, ERROR_TEXT_LIMIT),
                elide_to(&reading, ERROR_TEXT_LIMIT)
            ),
        )
    }

    /// A refusal naming what was expected and what arrived.
    /// Drops one thing the file states that this reader cannot keep.
    ///
    /// A CBlock is read for what it says. A tag it spells in a way the core
    /// cannot store, a constraint naming a tag its own vocabulary never
    /// declared, a mapping to a type nothing listed - each is dropped with a
    /// warning naming it and the byte it was read at, and the rest of the
    /// file is still a dictionary. Refusing a document of several thousand
    /// definitions over one of them would be refusing every definition
    /// nothing is wrong with.
    ///
    /// The warning is the refusal this reader would otherwise have raised,
    /// word for word, so what a log says and what an error would have said
    /// are one sentence written once.
    fn dropped(&self, error: &Error, kept: impl Display) {
        log::warn!("{}{error}; {kept}", self.prefix);
    }

    /// The line and column `position` falls on, one-based, for a warning to
    /// name beside the byte: a reader finds a line in a file that is
    /// megabytes of them, and a byte only through a tool. The newlines are
    /// indexed once, on the first warning, and never for a document that
    /// raises none.
    fn located(&self, position: usize) -> (usize, usize) {
        let lines = self.lines.get_or_init(|| {
            self.bytes
                .iter()
                .enumerate()
                .filter(|(_, byte)| **byte == b'\n')
                .map(|(at, _)| at)
                .collect()
        });
        let position = position.min(self.bytes.len());
        let line = lines.partition_point(|newline| *newline < position);
        let start = line
            .checked_sub(1)
            .map_or(0, |previous| lines[previous] + 1);
        (line + 1, position - start + 1)
    }

    /// A refusal at one byte of the document, its line and column named
    /// beside the byte.
    fn refusal_at(&self, position: usize, reason: SmolStr) -> Error {
        let (line, column) = self.located(position);
        refusal(
            position,
            format_smolstr!("line {line}, column {column}: {reason}"),
        )
    }

    fn refused(&self, expected: impl Display, got: impl Display) -> Error {
        self.refusal_at(self.position(), expected_got(expected, got))
    }

    /// The same refusal, quoting the element the file spells it in.
    fn refused_in(
        &self,
        element: &BytesStart<'_>,
        expected: impl Display,
        got: impl Display,
    ) -> Error {
        let spelling = spelling(element);
        self.refused(
            expected,
            format_args!("{got} in {:?}", elide_to(&spelling, ERROR_TEXT_LIMIT)),
        )
    }

    /// A refusal over an element the document never closed.
    ///
    /// Every loop below reads to its own end tag, and a reader answers no
    /// error for an element left open, so a document cut short - a partial
    /// download, a half-written file - would otherwise read as a smaller
    /// dictionary than the file it came from.
    fn unclosed(&self, name: &str) -> Error {
        self.refused(
            format_args!("a closed <{name}>"),
            format_args!(
                "the end of the document, reading {:?}",
                elide_to(&self.reading(), ERROR_TEXT_LIMIT)
            ),
        )
    }

    /// The one refusal a nested grammar with no counter raises.
    ///
    /// Two arms reach it - a grammar the file closed empty, and one whose
    /// children were all read before the grouping asked for the first - and
    /// they are the same statement about the same file.
    fn counterless(&self, msgtype: &str) -> Error {
        self.refused(
            "a nested grammar with a counter",
            format_args!(
                "an empty one in message {:?}",
                elide_to(msgtype, ERROR_TEXT_LIMIT)
            ),
        )
    }

    /// A refusal the core raised, behind what this file was reading.
    ///
    /// The core states what it would not store and this states which of the
    /// file's declarations asked it to, so neither half has to be guessed at.
    /// The core's own sentence crosses whole: it bounds the caller text it
    /// interpolates itself, so it says what it would not store rather than
    /// half of it.
    fn refused_by(&self, reading: impl Display, error: &Error) -> Error {
        self.refusal_at(self.position(), format_smolstr!("{reading}: {error}"))
    }
}

/// The datatype one CBlock type word names, or `None` for a word nothing
/// reads.
///
/// `float` is read here rather than through the schema grammar, because the
/// grammar reads that word as SQL does - a 32-bit float - where a CBlock
/// means FIX by it: the family `Qty`, `Price` and `Amt` derive from, which
/// states no width and which this crate types float64, as its logical names
/// for that family do. Every other word resolves through the grammar and the
/// FIX logical names behind it, folding case and separators as they do, so
/// the seven other CBlock types read as they always did and a real export
/// spelling a FIX datatype - `LocalMktDate`, `MonthYear`, `data`, `SeqNum` -
/// or a datatype name outright reads as what it names.
fn cblock_type(word: &str) -> Option<DataType> {
    if crate::implementer::folds_equal(word.trim(), "float") {
        return Some(DataType::Float64);
    }
    DataType::from_str(word).ok()
}

/// A refusal at one byte of the document.
fn refusal(position: usize, reason: SmolStr) -> Error {
    Error::Parse {
        target: TARGET,
        position,
        reason,
    }
}

/// One element as the file spells it: its name and every attribute.
///
/// A refusal quotes the source rather than describing it, so the declaration
/// it names is one search away in a file that is megabytes of them. The
/// closing form is this parser's, because an empty element's own `/` is not
/// in what the reader answers - only the space in front of it, which goes.
fn spelling(element: &BytesStart<'_>) -> String {
    format!("<{}>", String::from_utf8_lossy(element).trim_end())
}

/// The tag one mapping expression refers to, where it refers to one and
/// computes nothing.
///
/// `$602` is tag 602 and is the whole expression, so the name it is mapped to
/// is another spelling of that tag. Every other form mentions a tag without
/// being its name: `lookup("SecurityIDSource", $603)` decodes the value tag
/// 603 carries, `$603 = "4"` tests it, and a bare word refers to something
/// this file names elsewhere. None of them is a tag, and this layer holds no
/// evaluator to say what they would come to.
///
/// Trimmed, because a CBlock leaves the space it wrapped an attribute with -
/// `value="$609 "` is tag 609 written by an editor.
fn referenced(value: &str) -> Option<i32> {
    super::field::parse_tag(value.trim().strip_prefix('$')?)
}

/// One description as a single line of prose.
///
/// A stored description holds no control character, and a CBlock wraps a long
/// one over several indented lines, so every run of whitespace and control
/// characters becomes one space and the ends are dropped. What the file wrote
/// as layout was layout; every word it wrote survives.
fn single_line(text: &str) -> String {
    let mut held = String::with_capacity(text.len());
    for word in text.split(|character: char| character.is_whitespace() || character.is_control()) {
        if word.is_empty() {
            continue;
        }
        if !held.is_empty() {
            held.push(' ');
        }
        held.push_str(word);
    }
    held
}

/// The name the catalog holds the message of wire type `wire` under.
///
/// Tag 35's own code set names the type where it spells a name a store can
/// file, lower-cased; the wire value's own name
/// ([`derived_name`](super::msgtype::derived_name), `message` and its bytes
/// in hex) stands in where it does not.
///
/// **The wire value decides, never the derived name.** The name is
/// lower-cased out of a spelling the file chose, so two case-bearing types
/// can reach one spelling - a dialect naming `B` "News" and `b` "news"
/// derives `news` twice - and folding on that would merge two messages the
/// file was explicit about. A held entry under another wire value, or one
/// that is no message at all, is therefore a different definition wearing
/// this name, and this type takes the value-derived name no spelling can
/// contend. A held entry under this wire value is this same message, bound
/// by another grammar, and keeps the name so the second binding folds.
fn message_name(registry: &FixRegistry, wire: &str) -> String {
    let derived = || super::msgtype::derived_name(wire);
    let named = registry
        .get_field_by_tag(MSGTYPE_TAG_NAME.0)
        .and_then(|field| registry.codeset_of(field))
        .and_then(|set| set.code_name(wire))
        .filter(|name| {
            name.bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
        })
        .filter(|name| *name != wire)
        .map_or_else(derived, str::to_ascii_lowercase);
    match registry.get_definition(crate::FixCategory::Components, &named) {
        Some(entry) if FixField::new(entry).msgtype() != Some(wire) => {
            log::debug!(
                "message type {wire:?} takes {:?}: {named:?} names message type {:?}",
                derived(),
                FixField::new(entry).msgtype().unwrap_or_default()
            );
            derived()
        }
        _ => named,
    }
}

/// Stores a named definition, or answers the one the catalog holds of its
/// structure.
///
/// A definition is its [structure](super::catalog::structural_key): the
/// members a grammar states in order, each its tag or the field it reads, a
/// nested group its counter and the structure it repeats. A definition the
/// catalog holds with that structure is this one, under this name or any
/// other and however the grammar spelled it or how strictly it stated a
/// member: two grammars declaring one structure declare one definition, the
/// first declaration's name and spelling hold, a member one of them states
/// as required and the other does not is nullable, and the member reads the
/// held name. What is answered is the held definition as the occurrence the
/// member holds, so the member restates exactly what the catalog holds.
///
/// A name the catalog holds with another structure, where no definition of
/// this structure is held, is split under the message it was read in,
/// `{name}_{message}`: the split says where it came from, and a message is
/// one wire type, so two bindings of one type that declare it alike reach
/// one split rather than two. A message declaring one name in several
/// shapes - its parties at the root and again inside its legs, or two
/// bindings of the type stating it two ways - takes a split per shape in the
/// order it declares them, `{name}_{message}_2`, `_3`: a shape is one
/// definition and two shapes are two, and nothing already written is
/// rewritten under a member that has read it. Every definition written is
/// recorded in `written`, so a message that fails to catalogue takes them
/// back with it; a held definition relaxed for one stays relaxed, which
/// widens what it admits and contradicts nothing, and every member the
/// message built before the relaxation reads it again
/// ([`catalog_members`]).
fn catalog_entry(
    registry: &mut FixRegistry,
    category: crate::FixCategory,
    mut field: Field,
    message: &str,
    written: &mut Vec<(crate::FixCategory, String)>,
    structures: &mut Structures,
) -> Result<Field> {
    let base = field.name().to_owned();
    for ordinal in 0_usize.. {
        match ordinal {
            0 => {}
            1 => field.set_name(format!("{base}_{message}")),
            ordinal => field.set_name(format!("{base}_{message}_{ordinal}")),
        }
        let held = registry.get_definition(category, field.name());
        if let Some(held) = held
            && restated(held, &field)
        {
            return Ok(occurrence(held, &field));
        }
        let taken = held.is_some();
        // Before a name of its own and before a split: a held definition of
        // this structure, under any name, is this one.
        if ordinal == 0
            && let Some(name) = structures.find(registry, category, &field)?
        {
            return reuse(registry, category, &name, &field, structures);
        }
        if !taken {
            break;
        }
    }
    registry.insert_definition(category, field.clone())?;
    written.push((category, field.name().to_owned()));
    structures.written(registry, category, &field)?;
    Ok(field)
}

/// The held definition as the occurrence a member of it holds: without the
/// derived tag only the definition carries, under the member's own
/// nullability. A reference is proven against exactly what the catalog
/// holds, so this is what a member reading a held definition restates.
fn occurrence(held: &Field, member: &Field) -> Field {
    let mut occurrence = held.clone();
    occurrence.remove_metadata(super::field::TAG_KEY);
    occurrence.set_nullable(member.is_nullable());
    occurrence
}

/// The structures the catalog holds, by [structure](super::catalog::structural_key), each the
/// name of the first definition stating it: what one parse asks, once per
/// group or component a grammar declares, which definition it declared.
///
/// Built from the catalog the first time it is asked and kept by the three
/// events that move it: a definition written is indexed under its structure,
/// a walk taken back takes its definitions out, and a second binding of a
/// wire type - which folds members into definitions it holds, so their
/// structures move - empties it, to be built again when next asked. A
/// message is its wire type's and never another's, so none is indexed.
/// Beside the index it counts the definitions a reuse relaxed, which is what
/// tells a walk that a record it built holds a copy the catalog no longer
/// does.
#[derive(Default)]
struct Structures {
    held: Option<std::collections::HashMap<(crate::FixCategory, SmolStr), String>>,
    /// How many held definitions [`reuse`] relaxed so far: a record built
    /// before a later member relaxed a definition it reads holds the copy
    /// the catalog held then, so a walk reads its members again where the
    /// count moved under it.
    relaxed: usize,
}

impl Structures {
    /// The name of a held definition of `field`'s structure, where the
    /// catalog holds one.
    fn find(
        &mut self,
        registry: &FixRegistry,
        category: crate::FixCategory,
        field: &Field,
    ) -> Result<Option<String>> {
        if FixField::new(field).msgtype().is_some() {
            return Ok(None);
        }
        let lookup =
            |category: crate::FixCategory, name: &str| registry.get_definition(category, name);
        let mut memo = super::catalog::StructureMemo::new();
        let key = super::catalog::structural_key(field, &lookup, &mut memo)?;
        // A record stating no member says nothing two declarations share.
        if key == super::catalog::EMPTY_STRUCTURE {
            return Ok(None);
        }
        let held = match &mut self.held {
            Some(held) => held,
            None => {
                let mut built = std::collections::HashMap::new();
                for category in [crate::FixCategory::Components, crate::FixCategory::Groups] {
                    for definition in registry.definitions(category) {
                        if FixField::new(definition).msgtype().is_some() {
                            continue;
                        }
                        let key = super::catalog::structural_key(definition, &lookup, &mut memo)?;
                        if key != super::catalog::EMPTY_STRUCTURE {
                            built
                                .entry((category, key))
                                .or_insert_with(|| definition.name().to_owned());
                        }
                    }
                }
                self.held.insert(built)
            }
        };
        Ok(held.get(&(category, key)).cloned())
    }

    /// Indexes the definition `name` just written, stating `field`'s
    /// structure.
    fn written(
        &mut self,
        registry: &FixRegistry,
        category: crate::FixCategory,
        field: &Field,
    ) -> Result<()> {
        let Some(held) = &mut self.held else {
            return Ok(());
        };
        if FixField::new(field).msgtype().is_some() {
            return Ok(());
        }
        let lookup =
            |category: crate::FixCategory, name: &str| registry.get_definition(category, name);
        let key = super::catalog::structural_key(
            field,
            &lookup,
            &mut super::catalog::StructureMemo::new(),
        )?;
        if key != super::catalog::EMPTY_STRUCTURE {
            held.entry((category, key))
                .or_insert_with(|| field.name().to_owned());
        }
        Ok(())
    }

    /// Takes out the definitions a walk wrote and the catalog took back.
    fn forgotten(&mut self, written: &[(crate::FixCategory, String)]) {
        if let Some(held) = &mut self.held {
            held.retain(|(category, _), name| {
                !written
                    .iter()
                    .any(|(taken, forgotten)| taken == category && forgotten == name)
            });
        }
    }

    /// Empties the index, for a fold that moved what the catalog holds.
    fn moved(&mut self) {
        self.held = None;
    }
}

/// Reads `field` as the held definition `name`: the held definition relaxed
/// to what both declarations state, restated in the catalog where that
/// changed it - counted on `structures`, so the records built before it
/// read the definition again - and answered as the occurrence the member
/// holds.
fn reuse(
    registry: &mut FixRegistry,
    category: crate::FixCategory,
    name: &str,
    field: &Field,
    structures: &mut Structures,
) -> Result<Field> {
    let held = registry.definition(category, name)?.clone();
    let merged = super::catalog::fold_alike(&held, field)?;
    if merged != held {
        log::debug!(
            "{:?} relaxes the {} {name:?} it declares alike",
            field.name(),
            category.as_str()
        );
        registry.restate_definition(category, merged)?;
        structures.relaxed += 1;
    } else if field.name() != name {
        log::debug!(
            "{:?} reads the {} {name:?} it declares alike",
            field.name(),
            category.as_str()
        );
    }
    Ok(occurrence(registry.definition(category, name)?, field))
}

/// A member reading a group or a component, as the occurrence of the
/// definition the catalog holds now - what [`catalog_entry`] answers for a
/// held definition - read again where a later member relaxed it; a member
/// reading no definition as it is.
fn reread(registry: &FixRegistry, member: &Field) -> Result<Field> {
    let view = FixField::new(member);
    let (category, name) = if let Some(name) = view.group() {
        (crate::FixCategory::Groups, name.to_owned())
    } else if let Some(name) = view.component() {
        (crate::FixCategory::Components, name.to_owned())
    } else {
        return Ok(member.clone());
    };
    let mut reread = occurrence(registry.definition(category, &name)?, member);
    match category {
        crate::FixCategory::Groups => FixFieldMut::new(&mut reread).set_group(&name)?,
        _ => FixFieldMut::new(&mut reread).set_component(&name)?,
    }
    reread.set_name(member.name());
    Ok(reread)
}

/// Whether the held definition is `field` as the catalog stored it.
///
/// The catalog lays a definition out before it stores it - a reference
/// occurrence loses the derived tag only its target carries and every level
/// normalizes its identifiers - and then stamps the root with the tag it
/// derives from the name, which the field a grammar produced has not been
/// given. So the incoming field is laid out the same way and the two compare
/// on everything but that tag: a group two messages declare alike is one
/// group, never a split of itself.
fn restated(held: &Field, field: &Field) -> bool {
    let mut held = held.clone();
    held.remove_metadata(super::field::TAG_KEY);
    super::catalog::canonical_occurrences(field.clone(), true).is_ok_and(|field| held == field)
}

fn catalog_members(
    registry: &mut FixRegistry,
    mut field: Field,
    message: &str,
    written: &mut Vec<(crate::FixCategory, String)>,
    structures: &mut Structures,
) -> Result<Field> {
    match field.dtype() {
        DataType::Struct(children) => {
            let relaxed = structures.relaxed;
            let mut children = children
                .iter()
                .cloned()
                .map(|child| catalog_members(registry, child, message, written, structures))
                .collect::<Result<Vec<_>>>()?;
            // A member built before a later one relaxed a definition it
            // reads - the parties at the root, stated strictly, before the
            // lax statement inside the legs - holds the copy the catalog
            // held then, and a reference restates exactly what the catalog
            // holds: so where a relaxation landed under this record, every
            // member reading a definition reads it again as it is now.
            if structures.relaxed != relaxed {
                for child in &mut children {
                    *child = reread(registry, child)?;
                }
            }
            field.set_dtype(DataType::from(StructType::from_fields(children)?))?;
        }
        DataType::Serie(item) => {
            // A split renames the definition and never the member: the
            // message still holds `underlyings`, reading the definition split
            // for it, so every dialect's message names one group one way and
            // a fold meets the two readings as one member.
            let (member, occurrence) = (field.name().to_owned(), item.name().to_owned());
            let item = catalog_members(
                registry,
                item.as_ref().clone(),
                message,
                written,
                structures,
            )?;
            let mut item = catalog_entry(
                registry,
                crate::FixCategory::Components,
                item,
                message,
                written,
                structures,
            )?;
            let component = item.name().to_owned();
            FixFieldMut::new(&mut item).set_component(&component)?;
            item.set_name(occurrence);
            field.set_dtype(DataType::serie(item))?;
            FixFieldMut::new(&mut field).set_component(&component)?;
            field = catalog_entry(
                registry,
                crate::FixCategory::Groups,
                field,
                message,
                written,
                structures,
            )?;
            let name = field.name().to_owned();
            FixFieldMut::new(&mut field).set_group(&name)?;
            field.set_name(member);
        }
        _ => {
            // By identity where the child kept the dictionary's name, else by
            // tag: a duplicate constraint or a contended spelling renamed the
            // child, and the tag is what still names the field it constrains.
            let view = FixField::new(&field);
            let known = match (view.id()?, view.tag()?) {
                (Some(id), tag) => registry
                    .get_field_by_id(id)
                    .or_else(|| tag.and_then(|tag| registry.get_field_by_tag(tag))),
                (None, _) => None,
            };
            // A nested grammar retypes its counter int32 once it is read, so
            // a message bound before the group cloned the counter at the type
            // its word stated: the counter is the int32 now.
            if let Some(known) = known
                && (field.dtype() == known.dtype() || known.dtype() == &DataType::Int32)
            {
                // Maps and message codes can follow the grammar. Resolve
                // its earlier clone against the completed vocabulary.
                let name = field.name().to_owned();
                let nullable = field.is_nullable();
                field = known.clone();
                field.set_name(name);
                field.set_nullable(nullable);
                FixFieldMut::new(&mut field).set_field_ref(known.name())?;
            }
        }
    }
    Ok(field)
}

/// Whether one element carries this name, without its namespace prefix.
///
/// Compared as bytes rather than through a `String`: this runs on every event
/// of a document that is megabytes of them, and an allocation per name is an
/// allocation per element.
///
/// Generic over the opening and closing forms, which quick-xml types
/// separately and which this reads identically.
fn is_named(element: &impl Named, name: &[u8]) -> bool {
    element.local().as_ref() == name
}

/// One element's name, for the one caller that has to keep it.
fn local_name(element: &impl Named) -> Vec<u8> {
    element.local().as_ref().to_vec()
}

/// What an element of either form answers its own name with.
trait Named {
    fn local(&self) -> quick_xml::name::LocalName<'_>;
}

impl Named for BytesStart<'_> {
    fn local(&self) -> quick_xml::name::LocalName<'_> {
        self.local_name()
    }
}

impl Named for quick_xml::events::BytesEnd<'_> {
    fn local(&self) -> quick_xml::name::LocalName<'_> {
        self.local_name()
    }
}

/// How a CBlock spells one field: the `alt` its `vocabulary-tag` declared, or
/// the tag itself where it declared none.
///
/// The fallback is an identity rather than a guess, and the two writers of a
/// name are what make it one. [`Parse::push_tag`] stores `display` exactly
/// when the `alt` differs from the catalog name it folds to and names the
/// field that name; [`Parse::settle_names`] stores `display` whenever it
/// takes a contended spelling out of a name, and whenever a normalization's
/// spelling differs from the name it folds to. Either way a
/// `display` is the declared spelling and no `display` means the name already
/// *is* it. That coupling is load-bearing, because [`Parse::decodes`] orients
/// a map by comparing its name against this.
fn spelled(field: &Field) -> &str {
    field.display().unwrap_or_else(|| field.name()).trim()
}
