//! An Ullink CBlock configuration, read for the FIX definitions it declares.
//!
//! A `.cfb` is one counterparty's dictionary. It states at the top the FIX
//! version a dialect speaks, and then two things worth reading: a
//! `vocabulary` of tags, and a `grammar-binding` per message type describing
//! that message's tree. Everything else in the file describes the file, the
//! transcoding, or the plugin, and is skipped.
//!
//! # Two entry points, one parse
//!
//! [`FixRegistry::from_cfb_file`] answers the whole file: a dictionary of its
//! scalar fields with code metadata, components, groups and message definitions,
//! plus the branch record and the message roots. [`FixField::from_cfb_file`] answers
//! the vocabulary alone, in declaration order, and takes the branch name from
//! the file's own stem when the caller supplies none - which is what a reader
//! folding one counterparty's file into a dictionary through
//! [`FixRegistry::add_fields`] wants, and it loses the roots and the branch
//! record to say so. Both drive the same read, drop the same elements and
//! refuse the same two documents.
//!
//! # Two passes, and the second never invents a type
//!
//! `vocabulary` builds the dictionary; `grammar-binding` builds the message
//! roots out of it. A constraint borrows its resolved field and records its
//! own nullability. A nested grammar resolves its opening counter to int32,
//! retains that field, and adds a separately named List of components.
//!
//! # Only repeating groups are structure
//!
//! There is no component element and no reference mechanism. FIX component
//! blocks - `Instrument`, `UnderlyingInstrument`, `LegInstrument` - are
//! flattened into plain sibling constraints at the point of use and repeated
//! in full wherever they are needed. Nothing here recovers them, factors
//! repeated runs into shared sub-structs, or dedupes two grammars carrying
//! the same tag sequence. Every repeating group is a nested `grammar`, and
//! one recursive function reads all of them - nested grammars are siblings as
//! often as children. Its entries become named component definitions and its
//! collection becomes a group definition; both carry no synthetic FIX tag.
//! The count field remains a sibling immediately before the collection.
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
//! A CBlock's generic numeric types may disagree with the FIX dictionary.
//! For example, its `float` resolves to float32 while FIX `AvgPx` is float64.
//! The parser does not consult another registry or promote values by tag.
//! Merging incompatible datatypes or shared code sets fails atomically.
//! Replacing a referenced field also fails when its existing layouts would
//! become inconsistent; an unreferenced identity may be replaced directly.
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
//! fields with different tags. A dictionary indexes a name per branch, so
//! that spelling cannot name both there - and picking either would give a
//! reader a `HedgeCurrency` the file never said was the one.
//!
//! So it names neither. A tag whose `alt` another tag also declares, and a
//! tag whose `alt` is another tag's own decimal, are named by their own
//! decimal - the identity a tag declaring no `alt` already takes - and keep
//! the declared spelling as their `display`. Contended by the key a
//! dictionary indexes a name under rather than by the spelling, because that
//! key folds case and drops `_`, `-` and space: `Hedge_Currency` beside
//! `HedgeCurrency` is one name there and has to be one name here. That leaves
//! every tag named, always, and it is the same reading the normalization
//! section below applies to a name that means a tag only sometimes.
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
//! that its `tag-name` is another spelling of tag 602, so the spelling is
//! added to that tag as an alias. The vocabulary keeps the name.
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
//! is named by its own decimal tag, and the normalization is then the only
//! place the file says what it is called.
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
//! omits. Each becomes one code - the file's own spelling as its name, the
//! other spellings as aliases, the listing's wording as its description.
//!
//! A CBlock spells a type as the wire value and a qualifier, and the value is
//! the first word: `AR Inbound` and `AR Outbound` are tag 35 `AR` in the two
//! directions, `J Report` is `J` used as a report, `c SDR` and `c SLR` are
//! `c` used two ways. A FIX message type is alphanumeric and never holds a
//! space, so two declarations of one wire type are one code answering to
//! both spellings - and a message root keeps the whole spelling, because the
//! qualifier is what says which grammar the file bound. Resolving a root's
//! name through this code set is what joins the two: `AR Outbound` answers
//! `AR`. The complete wire token is retained without a width limit.
//!
//! A message type is a code of tag 35 and never a field, so a file that
//! declares no tag 35 keeps its types out of the dictionary rather than
//! inventing the field to hold them. What the file's own `map` for tag 35
//! said leads, because a map states a wire value.
//!
//! # What is dropped, and what is refused
//!
//! A CBlock is read for what it says. A real one is megabytes over hundreds
//! of thousands of elements, and one element this reader cannot make sense of
//! is one element: a tag spelled in a way the core cannot store, a constraint
//! naming a tag the file's own vocabulary never declared, a mapping to a type
//! nothing listed, a `fix-version` the version grammar cannot read. Each is
//! dropped, the rest of the file is still a dictionary, and what went is
//! logged at warn level.
//!
//! A dropped element carries the sentence a refusal would have: one
//! [`Error::Parse`] with `cfb` as its target, the byte the reader had reached
//! as its position, and a reason that quotes the document rather than
//! describing it - `expected a decimal tag, got "MsgType" in
//! "<vocabulary-tag name=\"MsgType\" type=\"string\">"`. One that comes from
//! the core rather than from the grammar - a description or a spelling that
//! cannot be stored, a code set that cannot be rendered, two declarations of
//! one tag - carries the core's own sentence behind the tag and the wording
//! that raised it. Every span quoted from the document is bounded and the
//! core's is bounded by the error contract it is written under, so a warning
//! never grows with the file. What each drop leaves behind is stated where it
//! is made: a tag whose wording will not store keeps the tag, a constraint
//! that dangles keeps its message, a group with no counter keeps its parent,
//! and a message the catalog will not hold keeps every field the file
//! declared.
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

use std::fmt::Display;

use quick_xml::Reader;
use quick_xml::events::{BytesStart, Event};
use smol_str::{SmolStr, format_smolstr};

use crate::text::{ERROR_TEXT_LIMIT, elide_to, expected_got};
use crate::{DataType, Error, Field, FixField, IOBase, Result, Url, Version};

use super::{FixBranch, FixCode, FixId, FixRegistry, MSGTYPE_TAG};

/// How deep a grammar may nest before the parse stops descending.
///
/// The deepest observed in production is five. The guard is well above that
/// and exists so a malformed or hostile file cannot recurse the stack away.
const MAX_DEPTH: usize = 32;

/// What this parser names itself in a refusal and in a warning.
const TARGET: &str = "cfb";

/// The eight types a `vocabulary-tag` may declare.
///
/// Seven resolve straight through the schema grammar's own logical names,
/// which fold case and drop separators. `utc-date` is the eighth and folds to
/// `utcdate`, a spelling the table now carries beside `utcdateonly` - added
/// there rather than mapped here, because a logical name belongs to the
/// registry that owns them.
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

/// The two domains a validity element may declare.
const DOMAINS: [&str; 2] = ["all-values", "ranges"];

impl FixRegistry {
    /// Parses an Ullink CBlock configuration into the vocabulary it declares
    /// and the message roots its grammar bindings describe.
    ///
    /// `branch` names the dialect, because a `.cfb` never names itself: the
    /// file states a version and a session but no name for the pair, so the
    /// caller supplies one. `None` reads it into the standard branch, which
    /// is right for a file read only for its vocabulary. The root element's
    /// `fix-version`, `sendercompid` and `targetcompid` become that branch's
    /// own record.
    ///
    /// The registry holds scalar wire fields under their tags and message
    /// roots under the Messages category. Nested grammars contribute Groups
    /// and Components definitions, with `fix:counter` linking each list to its
    /// ordinary int32 field. Enumerations stay in each field's `fix:codes`
    /// metadata. Named definitions carry no `fix:tag`.
    ///
    /// No seed is taken: this answers what one file says. Folding it into a
    /// dictionary that already exists is [`FixRegistry::add_fields`]'s job,
    /// and [`FixField::from_cfb_file`] is the door to take when the roots are
    /// not wanted.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Parse`] naming the byte position and quoting what the
    /// reader stopped on, for a document that is not well-formed XML or that
    /// stops with an element open. Everything else this reader cannot make
    /// sense of - an unreadable `fix-version`, a `type` outside the eight, a
    /// `domain` outside the two, a constraint whose tag misses the vocabulary,
    /// a nested grammar with no counter, nesting past the guard depth, a
    /// mapping to a type nothing listed, and anything the core will not store,
    /// from a spelling to a second declaration of one tag - is dropped with a
    /// warning naming it, and the rest of the file is still a dictionary. The
    /// warnings are `log` records at warn level; nothing is emitted unless the
    /// host installs a logger.
    pub fn from_cfb_file(
        handle: &dyn IOBase,
        branch: Option<&FixBranch>,
    ) -> Result<(Self, Vec<Field>)> {
        let bytes = handle.read_all_bytes()?;
        Parse::new(&bytes, branch).run()
    }
}

impl FixField<'_> {
    /// Reads an Ullink CBlock configuration for the vocabulary it declares.
    ///
    /// The dictionary half of [`FixRegistry::from_cfb_file`], answered on
    /// its own and in declaration order. Every field carries the `fix:tag` and
    /// `fix:branch` that key it and whatever code set the file's maps decode
    /// for it, which is what [`FixRegistry::add_fields`] needs to fold one
    /// counterparty's file into a dictionary that already exists.
    ///
    /// **`branch` names the dialect, and the file names it when the caller
    /// does not.** A CBlock states a version and a session but no name for the
    /// pair, so with none supplied the handle's own stem stands in:
    /// `s3://cblocks/MSFIX44.cfb` reads into the branch `msfix44`. A handle
    /// answering no URL at all has no stem and reads into the standard branch.
    ///
    /// A stem that is not a branch is refused rather than folded into one,
    /// because a dictionary keyed on a guess is worse than a refusal. That
    /// includes the stem of a [`Buffer`](crate::holder::Buffer), whose URL is
    /// an identity and not a location: bytes held in memory are named by the
    /// caller or not at all.
    ///
    /// The grammar bindings are still read and still validated, exactly as
    /// [`FixRegistry::from_cfb_file`] reads them, and their roots dropped: one
    /// parse, one set of drops and refusals, whichever entry point a caller
    /// takes.
    ///
    /// Two things a registry would hold are therefore not here - the message
    /// roots, and the branch record. A field stores its branch's *name* and
    /// never the version the root element declares, so a dictionary built
    /// from these fields alone knows the dialect by name and nothing else.
    /// Take [`FixRegistry::from_cfb_file`] when either matters.
    ///
    /// One difference is not a loss: a dictionary keeps one entry per
    /// identity, so a tag a file declares twice identically arrives twice
    /// here and once there.
    ///
    /// # Errors
    ///
    /// Returns what [`FixRegistry::from_cfb_file`] returns, and
    /// [`Error::Parse`] naming `fix branch` when the supplied name - or the
    /// stem standing in for it - is not one.
    pub fn from_cfb_file(handle: &dyn IOBase, branch: Option<&str>) -> Result<Vec<Field>> {
        let dialect = branch
            .or_else(|| handle.url().and_then(Url::stem))
            .map(FixBranch::from_str)
            .transpose()?;
        let bytes = handle.read_all_bytes()?;
        Parse::new(&bytes, dialect.as_ref()).fields()
    }
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
    /// The dialect as the caller named it, filled in from the root element.
    branch: FixBranch,
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
    roots: Vec<Field>,
    /// The names the normalization bindings spell for a tag, in declaration
    /// order.
    ///
    /// Collected rather than attached as they arrive, for the reason the
    /// message types are: a normalization is checked against the whole
    /// vocabulary - the tag it names, what that tag is already called, and
    /// what every other tag answers to - and a file is free to spell one
    /// before it declares the other.
    named: Vec<Spelled>,
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
    /// The byte the reader had reached when this declaration was read.
    ///
    /// Kept because the dictionary is built after the whole document is: a
    /// warning over two declarations of one tag names the declaration that
    /// raised it rather than the end of the file.
    position: usize,
    field: Field,
}

impl<'doc> Parse<'doc> {
    fn new(bytes: &'doc [u8], branch: Option<&FixBranch>) -> Self {
        // Text arrives exactly as the file wrote it. The reader's own trimming
        // is per event, and a description carrying an entity arrives as
        // several, so it would eat the space in front of `&lt;SOH&gt;` and
        // join two words. Layout is [`single_line`]'s question, and the only
        // text this reads is a description's.
        let reader = Reader::from_reader(bytes);
        Self {
            reader,
            bytes,
            branch: branch.cloned().unwrap_or(FixBranch::STANDARD),
            vocabulary: Vec::new(),
            positions: std::collections::HashMap::new(),
            msgtypes: Vec::new(),
            values: std::collections::HashMap::new(),
            spellings: std::collections::HashMap::new(),
            roots: Vec::new(),
            named: Vec::new(),
        }
    }

    /// The vocabulary as a dictionary, and the roots the bindings describe.
    fn run(mut self) -> Result<(FixRegistry, Vec<Field>)> {
        self.read()?;
        self.dictionary()
    }

    /// The vocabulary alone, in declaration order.
    ///
    /// Declaration order is what a caller folding one file into another wants
    /// and what a dictionary does not keep, so it is taken first. The
    /// dictionary is then built and dropped, because building it is the second
    /// half of what reading this file means: it is where a name or an identity
    /// the file declares twice is dropped, and both doors have to drop the
    /// same declarations.
    ///
    /// Two things a dictionary would hold are not here - the roots, and the
    /// branch record, which no field can carry. A third is a difference rather
    /// than a loss: a dictionary keeps one entry per identity, so a tag a file
    /// declares twice identically arrives twice here and once there.
    fn fields(mut self) -> Result<Vec<Field>> {
        self.read()?;
        let ordered: Vec<Field> = self
            .vocabulary
            .iter()
            .map(|held| held.field.clone())
            .collect();
        self.dictionary()?;
        Ok(ordered)
    }

    /// The vocabulary as a dictionary.
    ///
    /// Both terminals build it, because it is where the file's own entries are
    /// checked against each other rather than only against the grammar.
    fn dictionary(mut self) -> Result<(FixRegistry, Vec<Field>)> {
        let mut registry = FixRegistry::new();
        // The branch record first, and explicitly. A field's metadata carries
        // only its branch's *name* - the version is the dictionary's own
        // record of the dialect - so inserting fields alone
        // would register a nameless-versioned branch and lose what the root
        // element was read for. The standard branch declares no dialect, so a
        // file parsed without a name registers nothing.
        if !self.branch.is_standard() {
            let branch = self.branch.clone();
            if let Err(error) = registry.set_branch(branch) {
                // Located at the start, where the root element that declared
                // this branch is: it is the document's first element, and the
                // reader is at the end of the file by the time this runs.
                self.dropped(&refusal(
                    0,
                    format_smolstr!(
                        "branch {} at FIX {}: {error}",
                        self.branch,
                        self.branch.version()
                    ),
                ));
            }
        }
        for held in std::mem::take(&mut self.vocabulary) {
            let Declared {
                tag,
                position,
                field,
            } = held;
            // Read before the field moves: the warning names the entry the
            // file declared, which the registry no longer has to hand.
            let named = SmolStr::new(field.name());
            if let Err(error) = registry.insert(field) {
                self.dropped(&refusal(
                    position,
                    format_smolstr!(
                        "tag {tag} {:?}: {error}",
                        elide_to(&named, ERROR_TEXT_LIMIT)
                    ),
                ));
            }
        }
        let mut roots = Vec::with_capacity(self.roots.len());
        let held = std::mem::take(&mut self.roots);
        for root in held {
            let wire = msgtype_value(root.name()).to_owned();
            let named = SmolStr::new(root.name());
            match self.catalogued(&mut registry, root, &wire) {
                Ok(root) => roots.push(root),
                // One message the catalog will not hold is one message: the
                // dictionary keeps every field it read and every other root
                // the file bound.
                Err(error) => self.dropped(&refusal(
                    0,
                    format_smolstr!("message {:?}: {error}", elide_to(&named, ERROR_TEXT_LIMIT)),
                )),
            }
        }
        Ok((registry, roots))
    }

    /// One root as the catalog holds it: its members registered, its name
    /// resolved through tag 35's own code set, and the entry itself written.
    fn catalogued(&self, registry: &mut FixRegistry, root: Field, wire: &str) -> Result<Field> {
        let scope = wire
            .bytes()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        let mut root = catalog_members(registry, root, &scope)?;
        let held = root.clone();
        let named = registry
            .get_field_by_tag(MSGTYPE_TAG)
            .and_then(|field| field.as_fix().code_name(wire))
            .filter(|name| {
                name.bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
            })
            .filter(|name| *name != wire)
            .map_or_else(|| format!("message{scope}"), str::to_ascii_lowercase);
        root.set_name(named);
        root.as_fix_mut().set_branch(&self.branch)?;
        root.as_fix_mut().set_msgtype(wire)?;
        catalog_entry(registry, crate::FixCategory::Messages, root, &scope)?;
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
                    if is_named(&element, b"cplugin-configuration") {
                        self.read_root(&element)?;
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
                Ok(Event::Empty(element)) => {
                    if is_named(&element, b"cplugin-configuration") {
                        self.read_root(&element)?;
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
    /// another tag's own decimal, are therefore named by their own decimal -
    /// the same identity a tag declaring no `alt` already takes - and keep
    /// the declared spelling as their `display`, so nothing the file said is
    /// lost and [`spelled`] still answers what the counterparty calls them.
    ///
    /// Contended per branch, because a name is unique per branch: a venue's
    /// own 11024 and FIX's 44 never contend, and [`Parse::owner`] is what
    /// decides which of the two a tag lands in.
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
            let branch = self.owner(held.tag);
            let tags = claimed
                .entry(super::registry::name_key(&branch, held.field.name()))
                .or_default();
            if !tags.contains(&held.tag) {
                tags.push(held.tag);
            }
            decimals.insert(
                super::registry::name_key(&branch, &format_smolstr!("{}", held.tag)),
                held.tag,
            );
        }
        for at in 0..self.vocabulary.len() {
            let tag = self.vocabulary[at].tag;
            let key = super::registry::name_key(&self.owner(tag), self.vocabulary[at].field.name());
            let sharing = claimed.get(&key).cloned().unwrap_or_default();
            let contended =
                sharing.len() > 1 || decimals.get(&key).is_some_and(|held| *held != tag);
            if !contended {
                continue;
            }
            let position = self.vocabulary[at].position;
            let spelling = SmolStr::new(spelled(&self.vocabulary[at].field));
            let named = format_smolstr!("{tag}");
            let mut warnings: Vec<Error> = Vec::new();
            let field = &mut self.vocabulary[at].field;
            if spelling != named {
                if let Err(error) = field.set_display(spelling.as_str()) {
                    warnings.push(refusal(
                        position,
                        format_smolstr!(
                            "tag {tag} spelling {:?}: {error}",
                            elide_to(&spelling, ERROR_TEXT_LIMIT)
                        ),
                    ));
                }
            }
            field.set_name(named);
            // Two tags sharing a spelling name each other, so a reader holding
            // either reaches the other one the file said the same thing about.
            // Three cannot: an alternate identifier names one field, and three
            // would each claim the other two - so a spelling three tags share
            // links none of them, exactly as it names none of them.
            if sharing.len() == 2 {
                let other = sharing[usize::from(sharing[0] == tag)];
                if let Err(error) = field.as_fix_mut().set_tags(&[other]) {
                    warnings.push(refusal(
                        position,
                        format_smolstr!("tag {tag} beside tag {other}: {error}"),
                    ));
                }
            }
            for warning in &warnings {
                self.dropped(warning);
            }
        }
        Ok(())
    }

    /// Reads the branch record the root element carries.
    ///
    /// A CBlock is one counterparty's dictionary, and `fix-version` is what it
    /// declares about the dialect. It becomes the branch's version and never
    /// its name: a branch name must start with an ASCII letter, so `4.4` could
    /// not be one, which is what carrying both on one record ends the
    /// confusion about.
    ///
    /// `sendercompid` and `targetcompid` are read past. A branch is a
    /// dictionary, and which two parties spoke it is a fact about a run rather
    /// than about a vocabulary - the same file written from the other side
    /// declares the pair reversed and describes the same dialect.
    fn read_root(&mut self, element: &BytesStart<'_>) -> Result<()> {
        // An absent `fix-version` is the file saying nothing about the
        // dialect and keeps the caller's; one the version grammar cannot read
        // is the file saying something this cannot, and is dropped with a
        // warning, which keeps the caller's too - so what the dialect is
        // recorded as is always something somebody stated.
        let version = match self.attribute(element, "fix-version")? {
            // Trimmed, because attribute-value normalization turns a wrapped
            // line into a space and never drops one; quoted as the file wrote
            // it, because that is what a warning is for.
            Some(held) => match held.trim().parse::<Version>() {
                Ok(version) => version,
                Err(error) => {
                    self.dropped(&self.refused_in(
                        element,
                        "a FIX version",
                        format_args!("{:?}: {error}", elide_to(&held, ERROR_TEXT_LIMIT)),
                    ));
                    self.branch.version()
                }
            },
            None => self.branch.version(),
        };
        match FixBranch::from_parts(self.branch.name(), version) {
            Ok(branch) => self.branch = branch,
            Err(error) => self.dropped(&self.refused_by(
                format_args!("branch {} at FIX {version}", self.branch),
                &error,
            )),
        }
        Ok(())
    }

    /// Which branch owns one tag.
    ///
    /// A dialect redefines its own tags, never FIX's: `BeginString(8)` means
    /// the same thing in every counterparty's file, and only the
    /// user-defined range is a venue's to claim. So a CBlock's standard tags
    /// land in the standard branch and its custom ones in the named branch -
    /// which is also what the identity rule requires, since a non-standard
    /// branch may only claim that range.
    ///
    /// The consequence is worth knowing: a file read for a named dialect
    /// still contributes most of its vocabulary to the standard branch, and
    /// two counterparties' files merge there rather than each shadowing FIX.
    fn owner(&self, tag: i32) -> FixBranch {
        if (FixId::USER_TAG_MIN..FixId::USER_TAG_MAX).contains(&tag) {
            self.branch.clone()
        } else {
            FixBranch::STANDARD
        }
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
                Event::Text(text) if describing => {
                    let raw = String::from_utf8_lossy(text.as_ref()).into_owned();
                    let held = quick_xml::escape::unescape(&raw)
                        .map_err(|error| self.malformed(&error.to_string()))?;
                    described.push_str(&held);
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
        let Some(name) = self.attribute(element, "name")? else {
            self.dropped(&self.refused_in(
                element,
                "a vocabulary-tag naming its tag",
                "one without",
            ));
            return Ok(());
        };
        let Ok(tag) = name.parse::<i32>() else {
            self.dropped(&self.refused_in(
                element,
                "a decimal tag",
                format_args!("{:?}", elide_to(&name, ERROR_TEXT_LIMIT)),
            ));
            return Ok(());
        };
        let declared = self
            .attribute(element, "type")?
            .unwrap_or_else(|| "string".to_owned());
        if !TYPES.contains(&declared.as_str()) {
            self.dropped(&self.refused_in(
                element,
                format_args!("one of the eight CBlock types ({})", TYPES.join(", ")),
                format_args!("{:?}", elide_to(&declared, ERROR_TEXT_LIMIT)),
            ));
            return Ok(());
        }
        let dtype = match DataType::from_str(&declared) {
            Ok(dtype) => dtype,
            Err(error) => {
                self.dropped(&self.refused_by(format_args!("tag {tag} type {declared:?}"), &error));
                return Ok(());
            }
        };

        // The FIX name case-folded and nothing else, with the file's own
        // spelling kept beside it: name resolution folds ASCII case, so a
        // caller spelling it the file's way still resolves and nothing is lost.
        let alt = self.attribute(element, "alt")?;
        let spelling = alt.clone().unwrap_or_else(|| name.clone());
        let mut field = dtype.nullable_field(spelling.to_ascii_lowercase());
        // The file's own spelling first: `set_metadata` replaces the whole
        // snapshot, so anything written into the `fix:` namespace before it
        // would be replaced away.
        if let Some(alt) = alt.filter(|held| held != &held.to_ascii_lowercase()) {
            if let Err(error) = field.set_metadata([("display", alt.as_str())]) {
                self.dropped(&self.refused_by(
                    format_args!("tag {tag} spelling {:?}", elide_to(&alt, ERROR_TEXT_LIMIT)),
                    &error,
                ));
                return Ok(());
            }
        }
        let owner = self.owner(tag);
        if let Err(error) = field.as_fix_mut().set_id(&owner, tag) {
            self.dropped(&self.refused_by(format_args!("tag {tag} on branch {owner}"), &error));
            return Ok(());
        }
        if let Some(described) = described {
            if let Err(error) = field.as_fix_mut().set_description(described) {
                self.dropped(&self.refused_by(
                    format_args!(
                        "tag {tag} description {:?}",
                        elide_to(described, ERROR_TEXT_LIMIT)
                    ),
                    &error,
                ));
            }
        }
        self.positions.insert(tag, self.vocabulary.len());
        self.vocabulary.push(Declared {
            tag,
            position: self.position(),
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
                    let named = self.attribute(&element, "name")?;
                    let decodes = named.as_deref().and_then(|named| self.decodes(named));
                    // A map naming no tag is still read to its end: skipping
                    // the entries would leave the reader inside them.
                    let codes = self.read_map_entries(decodes.is_some_and(|(_, fix)| fix))?;
                    // The name is what resolved the tag, so a map that
                    // decodes one has one to quote in a refusal.
                    if let Some(((at, _), named)) = decodes.zip(named.as_deref()) {
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

    /// One map's entries, each oriented the way the map's name says.
    ///
    /// `fix` is what [`Parse::decodes`] resolved: the FIX way round reads
    /// `key` as the wire value, the UlMessage way reads `value` as it.
    fn read_map_entries(&mut self, fix: bool) -> Result<Vec<FixCode>> {
        let mut codes: Vec<FixCode> = Vec::new();
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
                    self.push_entry(&element, fix, &mut codes)?;
                }
                Event::Start(element) if is_named(&element, b"entry") => {
                    self.push_entry(&element, fix, &mut codes)?;
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
        Ok(codes)
    }

    /// One `entry` as a code, oriented the way the map's name says.
    ///
    /// An empty attribute says what an absent one says, so both drop the
    /// entry. Everything else is kept, because a CBlock names one wire value
    /// twice routinely - `7` is both `accountiscarriedonnoncustomersmargined`
    /// and `accountishousetraderandcrossmargined` - and a set that took the
    /// first name and dropped the second would answer to one spelling fewer
    /// than the file declared. A second name for a value already held is what
    /// a code set calls an alias, so it is added as one.
    ///
    /// A spelling another code already answers to is the one thing that
    /// cannot be kept: two codes one spelling reaches resolve to nothing
    /// rather than to either, so the entry is dropped. That is a stricter
    /// test than [`FixCodes::render`](super::codes::FixCodes) refuses on -
    /// the whole fold rather than ASCII case - because render only has to
    /// keep a document readable and this has to keep it answerable.
    fn push_entry(
        &self,
        element: &BytesStart<'_>,
        fix: bool,
        codes: &mut Vec<FixCode>,
    ) -> Result<()> {
        let (Some(key), Some(held)) = (
            self.attribute(element, "key")?
                .filter(|held| !held.is_empty()),
            self.attribute(element, "value")?
                .filter(|held| !held.is_empty()),
        ) else {
            return Ok(());
        };
        let (value, name) = if fix { (key, held) } else { (held, key) };
        if codes.iter().any(|code| code.is_spelled(&name)) {
            return Ok(());
        }
        match codes.iter_mut().find(|code| code.value() == value) {
            Some(code) => code.push_alias(name),
            None => codes.push(FixCode::new(name, value)),
        }
        Ok(())
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
    /// can fold to one name across branches, and the one a map *spells* is
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
        let at = self.decoded(|held| crate::types::folds_equal(held.field.name(), named))?;
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

    /// Puts one code set on the vocabulary field its map decodes.
    ///
    /// An empty set is dropped rather than written: `set_codes` reads an empty
    /// slice as a removal, and a map declaring no entries is not the file
    /// saying the field has no codes.
    fn attach_codes(&mut self, at: usize, codes: &[FixCode], named: &str) -> Result<()> {
        if codes.is_empty() {
            return Ok(());
        }
        let tag = self.vocabulary[at].tag;
        if let Err(error) = self.vocabulary[at].field.as_fix_mut().set_codes(codes) {
            self.dropped(&self.refused_by(
                format_args!("map {:?} on tag {tag}", elide_to(named, ERROR_TEXT_LIMIT)),
                &error,
            ));
        }
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
                    let spelling = self.attribute(&element, "value")?;
                    let described = self.attribute(&element, "description")?;
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
                    let key = self.attribute(&element, "key")?;
                    let spelling = self.attribute(&element, "value")?;
                    if let Some(spelling) = spelling.filter(|held| !held.trim().is_empty()) {
                        let spelled = spelling.trim();
                        let Some(at) = self.values.get(msgtype_value(spelled)).copied() else {
                            self.dropped(&self.refused_in(
                                &element,
                                "a message type this file's listing declares",
                                format_args!("{:?}", elide_to(spelled, ERROR_TEXT_LIMIT)),
                            ));
                            buffer.clear();
                            continue;
                        };
                        // The entry's own spelling too: a listing that wrote
                        // `J Report` and a table that writes `J` are one type
                        // under two names, and both are names it answers to.
                        self.alias_msgtype(at, spelled);
                        if let Some(key) = key.filter(|held| !held.trim().is_empty()) {
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
    /// The value is [`msgtype_value`]'s and the name is the file's whole
    /// spelling, so two declarations of one wire type - `AR Inbound` and
    /// `AR Outbound` - are one code answering to both. A spelling the file
    /// states twice is one code, and the first wording it gave is the one it
    /// keeps.
    fn push_msgtype(&mut self, spelling: &str, described: Option<&str>) -> Option<usize> {
        let spelling = spelling.trim();
        let value = msgtype_value(spelling);
        let described = described.map(single_line).filter(|held| !held.is_empty());
        if let Some(at) = self.values.get(value).copied() {
            if self.msgtypes[at].description().is_none() {
                if let Some(described) = described {
                    self.msgtypes[at] = self.msgtypes[at].clone().with_description(described);
                }
            }
            self.alias_msgtype(at, spelling);
            return Some(at);
        }
        // Two spellings hashing to one value is a collision and not something
        // to resolve by picking one, so the second names nothing and is
        // dropped - the rule every contended spelling in this file is read
        // under.
        if let Some(taken) = self
            .spellings
            .get(&crate::types::normalized(spelling))
            .copied()
        {
            self.dropped(&self.refused(
                "a free message type value",
                format_args!(
                    "{:?} at {:?}, which {:?} holds",
                    elide_to(spelling, ERROR_TEXT_LIMIT),
                    value,
                    elide_to(self.msgtypes[taken].name(), ERROR_TEXT_LIMIT)
                ),
            ));
            return None;
        }
        let mut code = FixCode::new(spelling, value);
        if let Some(described) = described {
            code = code.with_description(described);
        }
        self.msgtypes.push(code);
        let at = self.msgtypes.len() - 1;
        self.values.insert(SmolStr::new(value), at);
        self.spellings
            .insert(crate::types::normalized(spelling), at);
        Some(at)
    }

    /// Adds one more spelling that reaches the message type at `at`.
    ///
    /// A spelling another code already answers to is dropped rather than
    /// added, because two codes one spelling reaches resolve to neither -
    /// the rule a map's entries are read under.
    fn alias_msgtype(&mut self, at: usize, spelling: &str) {
        let spelling = spelling.trim();
        if spelling.is_empty() {
            return;
        }
        let key = crate::types::normalized(spelling);
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
        let Some(at) = self.positions.get(&MSGTYPE_TAG).copied() else {
            return Ok(());
        };
        let mut codes = std::mem::take(&mut self.msgtypes);
        // What the file already said about tag 35 - a `map` naming it - leads,
        // because a map states the wire value and a message type is named by
        // the bridge; a spelling either already holds is not added twice.
        let held: Vec<FixCode> = self.vocabulary[at]
            .field
            .as_fix()
            .codes()
            .filter_map(std::result::Result::ok)
            .map(FixCode::from)
            .collect();
        codes.retain(|code| {
            !held
                .iter()
                .any(|kept| kept.value() == code.value() || kept.is_spelled(code.name()))
        });
        codes.splice(0..0, held);
        let position = self.vocabulary[at].position;
        if let Err(error) = self.vocabulary[at].field.as_fix_mut().set_codes(&codes) {
            self.dropped(&refusal(
                position,
                format_smolstr!("the message types tag {MSGTYPE_TAG} carries: {error}"),
            ));
        }
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
                            .attribute(element, "value")?
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
            .attribute(element, "tag-name")?
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
    /// - one another tag in the same branch already answers to, canonically
    ///   or as an alias. Two fields one spelling reaches resolve to neither,
    ///   so the second claim is dropped exactly as a map entry's second claim
    ///   on one name is - and a dictionary that refuses the whole file over
    ///   it would refuse a document nothing is wrong with.
    /// - one carrying the separator a stored alias list is rendered with,
    ///   which is the one spelling the core cannot hold.
    ///
    /// A branch is what scopes the third: a CBlock's standard tags land in
    /// the standard branch and its user-range tags in the named one, so two
    /// tags may share a spelling across the two without either losing it.
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
            let branch = self.owner(held.tag);
            let field = &held.field;
            let spellings = [field.name(), spelled(field)]
                .into_iter()
                .chain(field.as_fix().aliases());
            for spelling in spellings {
                claimed
                    .entry(super::registry::name_key(&branch, spelling))
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
            let key = super::registry::name_key(&self.owner(tag), &name);
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
            let mut aliases: Vec<SmolStr> = field.as_fix().aliases().map(SmolStr::new).collect();
            if aliases.iter().any(|held| name.eq_ignore_ascii_case(held)) {
                continue;
            }
            aliases.push(SmolStr::new(&name));
            if let Err(error) = field.as_fix_mut().set_aliases(&aliases) {
                self.dropped(&refusal(
                    position,
                    format_smolstr!(
                        "tag {tag} spelled {:?}: {error}",
                        elide_to(&name, ERROR_TEXT_LIMIT)
                    ),
                ));
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
        let declared = self.attribute(element, "type")?;
        // A file binds types its own listing sometimes omits, and a bound type
        // is one this dialect carries: the binding declares it too. The
        // unknown root is this crate's word for a binding that named nothing,
        // so it is not one of them.
        if let Some(declared) = declared.as_deref().filter(|held| !held.trim().is_empty()) {
            self.push_msgtype(declared, None);
        }
        let msgtype = declared.unwrap_or_else(|| super::build::UNKNOWN_MSGTYPE.to_owned());
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
                    match DataType::from_fields(children) {
                        Ok(dtype) => self.roots.push(dtype.required_field(msgtype.clone())),
                        // A root its own children will not make a struct of
                        // is one message dropped, not one file: the binding
                        // still declared the type, and every other message
                        // the file binds is untouched.
                        Err(error) => self.dropped(&self.refused_by(
                            format_args!("message {:?}", elide_to(&msgtype, ERROR_TEXT_LIMIT)),
                            &error,
                        )),
                    }
                }
                // An empty root grammar is an empty non-null struct rather
                // than a failure: the file bound the message and said it holds
                // nothing, which is a statement and not a defect.
                Event::Empty(element) if is_named(&element, b"grammar") => {
                    let root = DataType::from_fields([])?.required_field(msgtype.clone());
                    self.roots.push(root);
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
            self.dropped(&self.refused(
                format_args!("a grammar nested at most {MAX_DEPTH} deep"),
                format_args!(
                    "a deeper one in message {:?}",
                    elide_to(msgtype, ERROR_TEXT_LIMIT)
                ),
            ));
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
                        push_child(&mut children, field);
                    }
                }
                Event::Empty(element) if is_named(&element, b"tag-constraint") => {
                    if let Some(field) = self.read_constraint(&element, true, msgtype)? {
                        push_child(&mut children, field);
                    }
                }
                Event::Start(element) if is_named(&element, b"grammar") => {
                    let declared = self.attribute(&element, "rg-name")?;
                    let nested = self.read_grammar(depth + 1, msgtype)?;
                    if let Some((counter, group)) =
                        self.grouped(nested, msgtype, declared.as_deref())
                    {
                        push_child(&mut children, counter);
                        push_child(&mut children, group);
                    }
                }
                // A nested grammar with no children has no counter to name it,
                // so the group goes and the message keeps the rest.
                Event::Empty(element) if is_named(&element, b"grammar") => {
                    self.dropped(&self.counterless(msgtype));
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
    /// The scalar count precedes the named list; its item holds the members.
    fn grouped(
        &mut self,
        mut children: Vec<Field>,
        msgtype: &str,
        declared: Option<&str>,
    ) -> Option<(Field, Field)> {
        if children.is_empty() {
            self.dropped(&self.counterless(msgtype));
            return None;
        }
        let mut counter = children.remove(0);
        // A group whose first child is another grammar has no counter to name
        // it, so it is dropped while the parent keeps the rest.
        if matches!(counter.dtype(), DataType::List(_) | DataType::LargeList(_)) {
            self.dropped(&self.refused(
                "a nested grammar opening with its counter",
                format_args!(
                    "one opening with the group {:?} in message {:?}",
                    elide_to(counter.name(), ERROR_TEXT_LIMIT),
                    elide_to(msgtype, ERROR_TEXT_LIMIT)
                ),
            ));
            return None;
        }
        let named = SmolStr::new(counter.name());
        let dropping = |parse: &Self, error: &Error| {
            parse.dropped(&parse.refused_by(
                format_args!(
                    "group {:?} in message {:?}",
                    elide_to(&named, ERROR_TEXT_LIMIT),
                    elide_to(msgtype, ERROR_TEXT_LIMIT)
                ),
                error,
            ));
        };
        if let Err(error) = counter.set_dtype(DataType::Int32) {
            dropping(self, &error);
            return None;
        }
        let tag = match counter.as_fix().tag() {
            Ok(Some(tag)) => tag,
            Ok(None) => {
                self.dropped(&self.counterless(msgtype));
                return None;
            }
            Err(error) => {
                dropping(self, &error);
                return None;
            }
        };
        if let Some(position) = self.positions.get(&tag).copied() {
            if let Err(error) = self.vocabulary[position].field.set_dtype(DataType::Int32) {
                dropping(self, &error);
                return None;
            }
        }
        let mut name = declared.filter(|name| !name.is_empty()).map_or_else(
            || super::component::group_name(&counter).to_string(),
            |name| name.to_ascii_lowercase(),
        );
        if self
            .vocabulary
            .iter()
            .any(|field| field.field.name().eq_ignore_ascii_case(&name))
        {
            name.push_str("grp");
        }
        let mut entry = super::component::entry_name(&name).to_string();
        if self
            .vocabulary
            .iter()
            .any(|field| field.field.name().eq_ignore_ascii_case(&entry))
        {
            entry.push_str("component");
        }
        let built = || -> Result<(Field, Field)> {
            let branch = counter.as_fix().branch()?;
            let mut item = DataType::from_fields(children)?.required_field(entry.clone());
            item.as_fix_mut().set_branch(&branch)?;
            let mut group = DataType::list(item).nullable_field(name);
            group.set_nullable(counter.is_nullable());
            group.as_fix_mut().set_branch(&branch)?;
            group.as_fix_mut().set_counter(tag)?;
            group.as_fix_mut().set_component(&entry)?;
            Ok((counter.clone(), group))
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
        let Some(name) = self.attribute(element, "name")? else {
            self.dropped(&self.refused_in(
                element,
                "a tag-constraint naming its tag",
                format_args!(
                    "one without in message {:?}",
                    elide_to(msgtype, ERROR_TEXT_LIMIT)
                ),
            ));
            self.check_validity(closed)?;
            return Ok(None);
        };
        let Ok(tag) = name.parse::<i32>() else {
            self.dropped(&self.refused_in(
                element,
                "a decimal tag",
                format_args!(
                    "{:?} in message {:?}",
                    elide_to(&name, ERROR_TEXT_LIMIT),
                    elide_to(msgtype, ERROR_TEXT_LIMIT)
                ),
            ));
            self.check_validity(closed)?;
            return Ok(None);
        };
        let Some(held) = self
            .positions
            .get(&tag)
            .and_then(|at| self.vocabulary.get(*at))
            .map(|held| held.field.clone())
        else {
            self.dropped(&self.refused_in(
                element,
                "a tag this file's vocabulary declares",
                format_args!(
                    "tag {tag} in message {:?}",
                    elide_to(msgtype, ERROR_TEXT_LIMIT)
                ),
            ));
            self.check_validity(closed)?;
            return Ok(None);
        };
        // `required` is usually a flag but is sometimes a condition such as
        // `$59 = '6' and empty($126)`. An expression is read as not-required
        // rather than as a failure: this layer holds no evaluator, and a
        // conditionally required field is one that may be absent.
        let required = self
            .attribute(element, "required")?
            .is_some_and(|held| held.eq_ignore_ascii_case("true"));
        let mut field = held;
        field.set_nullable(!required);
        self.check_validity(closed)?;
        Ok(Some(field))
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
        let Some(domain) = self.attribute(element, "domain")? else {
            return Ok(());
        };
        if DOMAINS.contains(&domain.as_str()) {
            return Ok(());
        }
        // Every validity element is dropped anyway; the domain is read so
        // that one outside the two is a warning instead of a silence.
        self.dropped(&self.refused_in(
            element,
            format_args!("a domain of {} or {}", DOMAINS[0], DOMAINS[1]),
            format_args!("{:?}", elide_to(&domain, ERROR_TEXT_LIMIT)),
        ));
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
    fn attribute(&self, element: &BytesStart<'_>, name: &str) -> Result<Option<String>> {
        for attribute in element.attributes() {
            let attribute = attribute.map_err(|error| self.malformed(&error.to_string()))?;
            if attribute.key.local_name().as_ref() == name.as_bytes() {
                // Every CBlock declares `<?xml version="1.0"?>`, and the
                // normalization differs only for 1.1's extra control-character
                // entities, which none of the corpus carries.
                let held = attribute
                    .normalized_value(quick_xml::XmlVersion::Explicit1_0)
                    .map_err(|error| self.malformed(&error.to_string()))?;
                return Ok(Some(held.into_owned()));
            }
        }
        Ok(None)
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
    fn dropped(&self, error: &Error) {
        log::warn!("{error}");
    }

    fn refused(&self, expected: impl Display, got: impl Display) -> Error {
        refusal(self.position(), expected_got(expected, got))
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
        refusal(self.position(), format_smolstr!("{reading}: {error}"))
    }
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
    let held = value.trim().strip_prefix('$')?;
    if held.is_empty() || !held.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    held.parse().ok()
}

/// The wire value one declared message type carries.
///
/// A CBlock spells a message type as the wire value and a qualifier: `AR
/// Inbound` and `AR Outbound` are tag 35 `AR` in the two directions, `J
/// Report` is `J` used as a report, `c SDR` and `c SLR` are `c` used two
/// ways. The wire value is the first word - a FIX message type is
/// alphanumeric and never holds a space - and the qualifier says which
/// grammar the file binds under it, which is why a message root keeps the
/// whole spelling while the code takes the value and answers to both.
///
/// The full first word is retained without a datatype width limit.
fn msgtype_value(spelling: &str) -> &str {
    spelling.split_whitespace().next().unwrap_or(spelling)
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

/// Stores a named definition, qualifying distinct message contexts once.
fn catalog_entry(
    registry: &mut FixRegistry,
    category: crate::FixCategory,
    mut field: Field,
    scope: &str,
) -> Result<Field> {
    let branch = field.as_fix().branch()?;
    if let Some(held) = registry.get_definition(category, field.name(), Some(&branch)) {
        if held == &field {
            return Ok(field);
        }
        field.set_name(format!("{}{scope}", field.name()));
        if let Some(held) = registry.get_definition(category, field.name(), Some(&branch)) {
            if held != &field {
                return Err(Error::Conflict {
                    expected: "one CBlock definition per context",
                    actual: "conflicting definitions",
                    path: field.name().into(),
                });
            }
            return Ok(field);
        }
    }
    registry.insert_definition(category, field.clone())?;
    Ok(field)
}

fn catalog_members(registry: &mut FixRegistry, mut field: Field, scope: &str) -> Result<Field> {
    match field.dtype() {
        DataType::Struct(children) => {
            let children = children
                .iter()
                .cloned()
                .map(|child| catalog_members(registry, child, scope))
                .collect::<Result<Vec<_>>>()?;
            field.set_dtype(DataType::from_fields(children)?)?;
        }
        DataType::List(item) => {
            let item = catalog_members(registry, item.as_ref().clone(), scope)?;
            let mut item = catalog_entry(registry, crate::FixCategory::Components, item, scope)?;
            let component = item.name().to_owned();
            item.as_fix_mut().set_component(&component)?;
            field.set_dtype(DataType::list(item))?;
            field.as_fix_mut().set_component(&component)?;
            field = catalog_entry(registry, crate::FixCategory::Groups, field, scope)?;
            let name = field.name().to_owned();
            field.as_fix_mut().set_group(&name)?;
        }
        _ => {
            if let Some(known) = field
                .as_fix()
                .id()?
                .and_then(|id| registry.get_field_by_id(id))
            {
                if field.dtype() == known.dtype() {
                    // Maps and message codes can follow the grammar. Resolve
                    // its earlier clone against the completed vocabulary.
                    let name = field.name().to_owned();
                    let nullable = field.is_nullable();
                    field = known.clone();
                    field.set_name(name);
                    field.set_nullable(nullable);
                    field.as_fix_mut().set_field_ref(known.name())?;
                }
            }
        }
    }
    Ok(field)
}

/// Preserves duplicate constraints in wire order under distinct child names.
/// Their original `fix:tag` still identifies the wire field.
fn push_child(children: &mut Vec<Field>, mut field: Field) {
    if !children.iter().any(|held| held.name() == field.name()) {
        children.push(field);
        return;
    }
    let base = field.name().to_owned();
    for suffix in 2..u32::MAX {
        let candidate = format!("{base}{suffix}");
        if !children.iter().any(|held| held.name() == candidate) {
            field.set_name(candidate);
            children.push(field);
            return;
        }
    }
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
/// when the `alt` differs from its own lower-case form and names the field
/// that same lower-case form; [`Parse::settle_names`] stores `display`
/// whenever it takes a contended spelling out of a name. Either way a
/// `display` is the declared spelling and no `display` means the name already
/// *is* it. That coupling is load-bearing, because [`Parse::decodes`] orients
/// a map by comparing its name against this.
fn spelled(field: &Field) -> &str {
    field.display().unwrap_or_else(|| field.name()).trim()
}
