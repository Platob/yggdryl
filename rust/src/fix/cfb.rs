//! An Ullink CBlock configuration, read for the FIX definitions it declares.
//!
//! A `.cfb` is one counterparty's dictionary. It states at the top exactly
//! what a dialect declares - the FIX version it speaks and the session pair
//! it speaks it over - and then two things worth reading: a `vocabulary` of
//! tags, and a `grammar-binding` per message type describing that message's
//! tree. Everything else in the file describes the file, the transcoding, or
//! the plugin, and is skipped.
//!
//! # Two entry points, one parse
//!
//! [`FixRegistry::from_cfb`] answers the whole file: a dictionary of its
//! vocabulary, the branch record its root element declares, and the message
//! roots its grammar bindings describe. [`FixField::from_cfb_file`] answers
//! the vocabulary alone, in declaration order, and takes the branch name from
//! the file's own stem when the caller supplies none - which is what a reader
//! folding one counterparty's file into a dictionary through
//! [`FixRegistry::add_fields`] wants, and it loses the roots and the branch
//! record to say so. Both drive the same read and refuse the same documents.
//!
//! # Two passes, and the second never invents a type
//!
//! `vocabulary` builds the dictionary; `grammar-binding` builds the message
//! roots out of it. A grammar never edits a dictionary entry: it clones one
//! and overrides the clone's nullability, so a tag bound `required="true"` in
//! one message and `required="false"` in another yields two independent
//! fields off one entry.
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
//! often as children, so a two-level special case would be wrong.
//!
//! # What is lost, by name
//!
//! `part` (so a `header` constraint and a `body` constraint sit as siblings
//! in one flat struct), `activated`, `read-only`, `ref`, `checkordering`,
//! `rg-name` beyond naming its group, `condition` and every `expression`,
//! `regexp`, `domain`, every range and sentinel, every validity element,
//! `merge-mode` (so a file patching a base configuration is read standalone),
//! `message-types` and both mapping tables, `history`, `cvs-revision`, the
//! root's `description`, `normalization-binding`, `reject-binding`,
//! `flow-filter-binding`, `options`, `noe-normalization-binding`, and
//! the root's own `version`, `date` and `logs`.
//!
//! And one loss of a different kind. A CBlock's `float` and `integer` are the
//! schema grammar's generic answers and the wrong shape for FIX money and
//! sequence numbers - the logical-name table already spells `price` and `qty`
//! as `decimal64(18,8)` and `seqnum` as `int64`. But a `.cfb` says nothing
//! about which tag is money, and this never promotes by tag or consults a
//! seeded registry mid-parse. What that costs a caller depends on the verb
//! they fold with, and both outcomes are worth stating plainly: against a
//! dictionary seeded from the committed one, where tag 6 `AvgPx` is stored
//! `float64`, [`FixRegistry::add_fields`] **refuses** it, because a datatype
//! is never widened silently, while [`FixRegistry::insert`] **replaces** it
//! wholesale on an identity match.
//!
//! Nothing is inferred from a validity child either: a `regexp` pinning a
//! length does not become a fixed-width ascii, and a `domain="ranges"` does
//! not become an enum.

use quick_xml::Reader;
use quick_xml::events::{BytesStart, Event};
use smol_str::{SmolStr, format_smolstr};

use crate::{DataType, Error, Field, FixField, IOBase, Result, Url, Version};

use super::{FixBranch, FixCode, FixId, FixRegistry};

/// How deep a grammar may nest before the parse refuses.
///
/// The deepest observed in production is five. The guard is well above that
/// and exists so a malformed or hostile file cannot recurse the stack away.
const MAX_DEPTH: usize = 32;

/// What this parser names itself in a refusal.
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
    /// The registry half holds the vocabulary, each field with its `fix:tag`,
    /// insertable and writable like any other. The roots are message trees
    /// and carry no tag, so they cannot enter a registry - a message root is
    /// a struct named by a MsgType, and inventing a synthetic tag to key one
    /// would put something in the dictionary that is not a field.
    ///
    /// No seed is taken: this answers what one file says. Folding it into a
    /// dictionary that already exists is [`FixRegistry::add_fields`]'s job,
    /// and [`FixField::from_cfb_file`] is the door to take when the roots are
    /// not wanted.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Parse`] naming the byte position for a document that
    /// is not well-formed, a `type` outside the eight, a `domain` outside the
    /// two, a constraint whose tag misses the vocabulary, a nested grammar
    /// with no counter, or nesting past the guard depth.
    pub fn from_cfb(handle: &dyn IOBase, branch: Option<&FixBranch>) -> Result<(Self, Vec<Field>)> {
        let bytes = handle.read_all_bytes()?;
        Parse::new(&bytes, branch).run()
    }
}

impl FixField<'_> {
    /// Reads an Ullink CBlock configuration for the vocabulary it declares.
    ///
    /// The dictionary half of [`FixRegistry::from_cfb`], answered on its own
    /// and in declaration order. Every field carries the `fix:tag` and
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
    /// [`FixRegistry::from_cfb`] reads them, and their roots dropped: one
    /// parse, one set of refusals, whichever entry point a caller takes.
    ///
    /// Two things a registry would hold are therefore not here - the message
    /// roots, and the branch record. A field stores its branch's *name* and
    /// never the version and session pair the root element declares, so a
    /// dictionary built from these fields alone knows the dialect by name and
    /// nothing else. Take [`FixRegistry::from_cfb`] when either matters.
    ///
    /// One difference is not a loss: a dictionary keeps one entry per
    /// identity, so a tag a file declares twice identically arrives twice
    /// here and once there.
    ///
    /// # Errors
    ///
    /// Returns what [`FixRegistry::from_cfb`] returns, and [`Error::Parse`]
    /// naming `fix branch` when the supplied name - or the stem standing in
    /// for it - is not one.
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
    /// The dialect as the caller named it, filled in from the root element.
    branch: FixBranch,
    /// The vocabulary in declaration order, and where each tag sits in it.
    ///
    /// Indexed rather than scanned: a binding resolves every constraint it
    /// carries against the whole vocabulary, and both are thousands long in a
    /// real file - so a scan per constraint is the parse squared.
    vocabulary: Vec<(i32, Field)>,
    positions: std::collections::HashMap<i32, usize>,
    roots: Vec<Field>,
}

impl<'doc> Parse<'doc> {
    fn new(bytes: &'doc [u8], branch: Option<&FixBranch>) -> Self {
        let mut reader = Reader::from_reader(bytes);
        reader.config_mut().trim_text(true);
        Self {
            reader,
            branch: branch.cloned().unwrap_or(FixBranch::STANDARD),
            vocabulary: Vec::new(),
            positions: std::collections::HashMap::new(),
            roots: Vec::new(),
        }
    }

    /// The vocabulary as a dictionary, and the roots the bindings describe.
    fn run(mut self) -> Result<(FixRegistry, Vec<Field>)> {
        self.read()?;
        let roots = std::mem::take(&mut self.roots);
        Ok((self.dictionary()?, roots))
    }

    /// The vocabulary alone, in declaration order.
    ///
    /// Declaration order is what a caller folding one file into another wants
    /// and what a dictionary does not keep, so it is taken first. The
    /// dictionary is then built and dropped, because building it is the second
    /// half of what reading this file means: it is where a name or an identity
    /// the file declares twice is refused, and both doors have to refuse the
    /// same documents.
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
            .map(|(_, field)| field.clone())
            .collect();
        self.dictionary()?;
        Ok(ordered)
    }

    /// The vocabulary as a dictionary.
    ///
    /// Both terminals build it, because it is where the file's own entries are
    /// checked against each other rather than only against the grammar.
    fn dictionary(self) -> Result<FixRegistry> {
        let mut registry = FixRegistry::new();
        // The branch record first, and explicitly. A field's metadata carries
        // only its branch's *name* - the version and the session pair are the
        // dictionary's own record of the dialect - so inserting fields alone
        // would register a nameless-versioned branch and lose what the root
        // element was read for. The standard branch declares no dialect, so a
        // file parsed without a name registers nothing.
        if !self.branch.is_standard() {
            registry.set_branch(self.branch.clone())?;
        }
        for (_, field) in self.vocabulary {
            registry.insert(field)?;
        }
        Ok(registry)
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
        Ok(())
    }

    /// Reads the branch record the root element carries.
    ///
    /// A CBlock is one counterparty's dictionary, and this is what it declares
    /// about the session it is for. `fix-version` becomes the branch's version
    /// and never its name: a branch name must start with an ASCII letter, so
    /// `4.4` could not be one, which is what carrying both on one record ends
    /// the confusion about.
    ///
    /// The session pair is stored exactly as declared. The `type` attribute's
    /// `BuySide`/`SellSide` portion says which side wrote the file, so the
    /// counterparty sees the same pair reversed - which is why a reader
    /// matching a session tries both orders, and why nothing here reverses
    /// anything.
    fn read_root(&mut self, element: &BytesStart<'_>) -> Result<()> {
        let version = self
            .attribute(element, "fix-version")?
            .and_then(|held| held.parse::<Version>().ok())
            .unwrap_or(self.branch.version());
        let sender = self
            .attribute(element, "sendercompid")?
            .unwrap_or_else(|| self.branch.sender_comp_id().to_owned());
        let target = self
            .attribute(element, "targetcompid")?
            .unwrap_or_else(|| self.branch.target_comp_id().to_owned());
        self.branch = FixBranch::from_parts(self.branch.name(), version, target, sender)?;
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
                Event::Eof => break,
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
    /// be this parser saying something.
    fn read_description(&mut self) -> Result<Option<String>> {
        let mut buffer = Vec::new();
        // Accumulated rather than assigned: a reader splits text at every
        // entity boundary, so one description carrying `&lt;SOH&gt;` arrives
        // as several events and keeping the last would keep a fragment.
        let mut described = String::new();
        loop {
            let event = self
                .reader
                .read_event_into(&mut buffer)
                .map_err(|error| self.malformed(&error.to_string()))?;
            match event {
                Event::Eof => break,
                Event::Text(text) => {
                    let raw = String::from_utf8_lossy(text.as_ref()).into_owned();
                    let held = quick_xml::escape::unescape(&raw)
                        .map_err(|error| self.malformed(&error.to_string()))?;
                    described.push_str(&held);
                }
                Event::GeneralRef(held) => {
                    let raw = format!("&{};", String::from_utf8_lossy(held.as_ref()));
                    let held = quick_xml::escape::unescape(&raw)
                        .map_err(|error| self.malformed(&error.to_string()))?;
                    described.push_str(&held);
                }
                Event::End(element) if is_named(&element, b"vocabulary-tag") => break,
                _ => {}
            }
            buffer.clear();
        }
        Ok(Some(described).filter(|held| !held.trim().is_empty()))
    }

    /// One `vocabulary-tag` as a dictionary field.
    fn push_tag(&mut self, element: &BytesStart<'_>, described: Option<&str>) -> Result<()> {
        let Some(name) = self.attribute(element, "name")? else {
            return Err(self.refused("a vocabulary-tag with a name", "one without"));
        };
        let tag: i32 = name
            .parse()
            .map_err(|_| self.refused("a decimal tag", &name))?;
        let declared = self
            .attribute(element, "type")?
            .unwrap_or_else(|| "string".to_owned());
        if !TYPES.contains(&declared.as_str()) {
            return Err(self.refused("one of the eight CBlock types", &declared));
        }
        let dtype = DataType::from_str(&declared)
            .map_err(|_| self.refused("a resolvable CBlock type", &declared))?;

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
            field.set_metadata([("display", alt.as_str())])?;
        }
        field.as_fix_mut().set_id(&self.owner(tag), tag)?;
        if let Some(described) = described.filter(|held| !held.trim().is_empty()) {
            field.as_fix_mut().set_description(described)?;
        }
        self.positions.insert(tag, self.vocabulary.len());
        self.vocabulary.push((tag, field));
        Ok(())
    }

    /// Reads every `map` into the code set of the tag it decodes.
    ///
    /// A map is named for the vocabulary tag it decodes - `ADVSIDE` decodes
    /// `AdvSide` - so the name resolves through the vocabulary this file has
    /// already read, and a map naming no tag is skipped rather than refused:
    /// a CBlock maps things that are not fields.
    fn read_maps(&mut self) -> Result<()> {
        let mut buffer = Vec::new();
        let mut depth = 0_usize;
        loop {
            let event = self
                .reader
                .read_event_into(&mut buffer)
                .map_err(|error| self.malformed(&error.to_string()))?;
            match event {
                Event::Eof => break,
                Event::Start(element) if is_named(&element, b"map") => {
                    let named = self.attribute(&element, "name")?;
                    let codes = self.read_map_entries()?;
                    self.attach_codes(named.as_deref(), codes)?;
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

    /// One map's entries, each oriented into a value and a name.
    fn read_map_entries(&mut self) -> Result<Vec<FixCode>> {
        let mut codes: Vec<FixCode> = Vec::new();
        let mut buffer = Vec::new();
        let mut depth = 0_usize;
        loop {
            let event = self
                .reader
                .read_event_into(&mut buffer)
                .map_err(|error| self.malformed(&error.to_string()))?;
            match event {
                Event::Eof => break,
                Event::Empty(element) if is_named(&element, b"entry") => {
                    self.push_entry(&element, &mut codes)?;
                }
                Event::Start(element) if is_named(&element, b"entry") => {
                    self.push_entry(&element, &mut codes)?;
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

    /// One `entry` as a code, oriented and deduplicated by its wire value.
    fn push_entry(&self, element: &BytesStart<'_>, codes: &mut Vec<FixCode>) -> Result<()> {
        let (Some(key), Some(held)) = (
            self.attribute(element, "key")?,
            self.attribute(element, "value")?,
        ) else {
            return Ok(());
        };
        let (value, name) = oriented(&key, &held);
        if !codes.iter().any(|code| code.value() == value) {
            codes.push(FixCode::new(name, value));
        }
        Ok(())
    }

    /// Puts one code set on the vocabulary field its map is named for.
    fn attach_codes(&mut self, named: Option<&str>, codes: Vec<FixCode>) -> Result<()> {
        let (Some(named), false) = (named, codes.is_empty()) else {
            return Ok(());
        };
        let Some(at) = self
            .vocabulary
            .iter()
            .position(|(_, field)| crate::types::folds_equal(field.name(), named))
        else {
            return Ok(());
        };
        self.vocabulary[at].1.as_fix_mut().set_codes(&codes)?;
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
        let msgtype = self
            .attribute(element, "type")?
            .unwrap_or_else(|| super::build::UNKNOWN_MSGTYPE.to_owned());
        let mut buffer = Vec::new();
        loop {
            let event = self
                .reader
                .read_event_into(&mut buffer)
                .map_err(|error| self.malformed(&error.to_string()))?;
            match event {
                Event::Eof => break,
                Event::Start(element) if is_named(&element, b"grammar") => {
                    let children = self.read_grammar(1)?;
                    let root = DataType::from_fields(children)?.required_field(msgtype.clone());
                    self.roots.push(root);
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
    fn read_grammar(&mut self, depth: usize) -> Result<Vec<Field>> {
        if depth > MAX_DEPTH {
            return Err(self.refused(
                format_args!("a grammar nested at most {MAX_DEPTH} deep"),
                "a deeper one",
            ));
        }
        let mut children: Vec<Field> = Vec::new();
        let mut buffer = Vec::new();
        loop {
            let event = self
                .reader
                .read_event_into(&mut buffer)
                .map_err(|error| self.malformed(&error.to_string()))?;
            match event {
                Event::Eof => break,
                Event::Start(element) if is_named(&element, b"tag-constraint") => {
                    let field = self.read_constraint(&element, false)?;
                    push_child(&mut children, field);
                }
                Event::Empty(element) if is_named(&element, b"tag-constraint") => {
                    let field = self.read_constraint(&element, true)?;
                    push_child(&mut children, field);
                }
                Event::Start(element) if is_named(&element, b"grammar") => {
                    let nested = self.read_grammar(depth + 1)?;
                    if let Some(group) = self.grouped(nested)? {
                        push_child(&mut children, group);
                    }
                }
                // A nested grammar with no children has no counter to name it.
                Event::Empty(element) if is_named(&element, b"grammar") => {
                    return Err(self.refused("a nested grammar with a counter", "an empty one"));
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
    /// The first child is the counter and is consumed rather than emitted: the
    /// group takes that counter's name, tag and description, while the
    /// counter's own integer type is discarded because a list's length already
    /// carries it. The item struct holds everything after it.
    fn grouped(&mut self, mut children: Vec<Field>) -> Result<Option<Field>> {
        if children.is_empty() {
            return Err(self.refused("a nested grammar with a counter", "an empty one"));
        }
        let counter = children.remove(0);
        // A group whose first child is another grammar has no counter to name
        // it, so it is skipped with a refusal while the parent keeps the rest.
        if matches!(counter.dtype(), DataType::List(_) | DataType::LargeList(_)) {
            return Err(self.refused(
                "a nested grammar opening with its counter",
                "one opening with another grammar",
            ));
        }
        let item = DataType::from_fields(children)?.required_field("item");
        let mut group = DataType::list(item).nullable_field(counter.name());
        group.set_nullable(counter.is_nullable());
        group.set_metadata(counter.as_metadata().iter())?;
        Ok(Some(group))
    }

    /// One `tag-constraint` as a leaf field.
    ///
    /// It resolves its tag in this file's own vocabulary, clones that entry
    /// and overrides only the clone's nullability - so the dictionary stays
    /// immutable and one entry serves every message that binds it.
    fn read_constraint(&mut self, element: &BytesStart<'_>, closed: bool) -> Result<Field> {
        let Some(name) = self.attribute(element, "name")? else {
            return Err(self.refused("a tag-constraint with a name", "one without"));
        };
        let tag: i32 = name
            .parse()
            .map_err(|_| self.refused("a decimal tag", &name))?;
        let Some(held) = self
            .positions
            .get(&tag)
            .and_then(|at| self.vocabulary.get(*at))
            .map(|(_, field)| field.clone())
        else {
            return Err(self.refused(
                "a tag this file's vocabulary declares",
                format_args!("tag {tag}"),
            ));
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
        Ok(field)
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
                Event::Eof => break,
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
        Err(self.refused("a domain of all-values or ranges", &domain))
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
                Event::Eof => break,
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

    /// A refusal naming the byte position the reader has reached.
    fn malformed(&self, reason: &str) -> Error {
        Error::Parse {
            target: TARGET,
            position: self.reader.buffer_position() as usize,
            reason: SmolStr::new(reason),
        }
    }

    /// A refusal naming what was expected and what arrived.
    fn refused(&self, expected: impl std::fmt::Display, got: impl std::fmt::Display) -> Error {
        Error::Parse {
            target: TARGET,
            position: self.reader.buffer_position() as usize,
            reason: format_smolstr!("expected {expected}, got {got}"),
        }
    }
}

/// Adds one child, disambiguating a name a sibling already took.
///
/// A duplicate tag at one level is possible because `part` is dropped, and a
/// tag bound in both `header` and `body` becomes two siblings. Neither an
/// error nor a deduplication: both are kept in document order and the later
/// one's name gains a numeric suffix, while its `fix:tag` stays identical so
/// the tag is still what recovers it.
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

/// One map entry oriented into the wire value and the name for it.
///
/// A CBlock writes `key="buy" value="B"`, so the name keys and the value is
/// the value - which is the order a code set wants. Not every map is written
/// that way, and one written the other way round would put `B` in a name and
/// `buy` on the wire.
///
/// The side that looks like a wire value decides. A FIX value is short and
/// has no word shape: `1`, `B`, `99`, `FXSPOT`. A symbolic name is longer and
/// reads as words. So the shorter side is the value, and where neither is
/// clearly shorter the declared order stands, because the declared order is
/// the documented one and a guess is worse than a convention.
fn oriented<'entry>(key: &'entry str, value: &'entry str) -> (&'entry str, &'entry str) {
    if value.len() <= key.len() {
        (value, key)
    } else {
        (key, value)
    }
}
