//! How a value of one field is restated, as one expression over the message.
//!
//! The specification retires a field or a value and says what stands in for
//! it: `Rule80A(47)` became `OrderCapacity(528)` beside
//! `OrderRestrictions(529)`, the partial-fill values of `ExecType(150)`
//! folded into `Trade`, `ExecBroker(76)` became one `Parties` occurrence
//! with role `1`. Those rules are facts about the field being restated, so
//! they travel on it: a registry adds or edits one by editing metadata, and
//! nothing in Rust holds a table of them.
//!
//! `FIX:replacements` is that document: one [canonical
//! document](super::document) of entries in **document order**, read
//! borrowed. Order is semantic - the first entry whose condition a message
//! meets answers, so a catch-all entry stating no condition comes last.
//!
//! # One rule is one plan
//!
//! Each entry states a [`Plan`](crate::Plan) of the crate's own
//! [expression](crate::expression) grammar and nothing else: its `select`
//! names the columns the rule fills and the terms they take, and its `where`
//! is the condition the message must meet. `FIX:derivation` already spells a
//! derived column as one [`Term`](crate::expression::Term); this is the same
//! vocabulary with a target list and a condition, so the crate holds one
//! expression grammar and no second one for replacement rules.
//!
//! The whole of the old fill vocabulary is ordinary terms. A constant is a
//! literal, the source's own value is the column naming it, another field's
//! value is that column, a join is [`concat`](crate::expression::Function),
//! and one occurrence of a repeating group is a list of one struct - so
//! `ExecBroker(76)` reads:
//!
//! ```text
//! select [{partyid: execbroker, partyrole: '1'}] as parties
//!   where execbroker is not null
//! ```
//!
//! What a reader does with an entry - matching the condition, refusing to
//! overwrite a stated value, merging a group occurrence - is the reader's;
//! this module owns what the document says and the one text it says it in.

use std::iter::FusedIterator;

use smol_str::{SmolStr, format_smolstr};

use super::document::{Cursor, Refusal, Scan, Writer, decode_text};
use crate::{Error, Plan, Result};

/// What the document is called for every refusal it raises.
const TARGET: &str = "fix replacements";

/// The rule itself: one plan, as its canonical text.
const PLAN: &str = "plan";
/// The specification's own wording of the mapping.
const DOC: &str = "doc";

/// The keys one entry may state, in the order it states them.
///
/// `plan` leads because it is the entry: a reader that has read it has read
/// the rule, and `doc` is what a person reads beside it.
pub(super) const KEYS: [&str; 2] = [PLAN, DOC];

/// One rule restating a value of the field carrying it, as a caller states
/// it.
///
/// The borrowed [`FixReplacementEntry`] is what a read answers; this is what
/// a writer hands [`FixFieldMut::set_replacements`](crate::FixFieldMut).
///
/// ```
/// use yggdryl::Plan;
/// use yggdryl::fix::FixReplacement;
///
/// # fn main() -> yggdryl::Result<()> {
/// // Rule80A(47) `A` is an agency order: OrderCapacity(528) takes `A`.
/// let agency = FixReplacement::new(
///     "select 'A' as ordercapacity where rule80a = 'A'".parse::<Plan>()?,
/// );
/// assert_eq!(agency.plan().to_string(), "select 'A' as ordercapacity where rule80a = 'A'");
/// assert_eq!(agency.doc(), None);
/// # Ok(())
/// # }
/// ```
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FixReplacement {
    plan: Plan,
    doc: Option<SmolStr>,
}

impl FixReplacement {
    /// Builds one rule from the plan that states it.
    #[must_use]
    pub const fn new(plan: Plan) -> Self {
        Self { plan, doc: None }
    }

    /// Sets the specification's own wording of the mapping.
    #[must_use]
    pub fn with_doc(mut self, doc: impl Into<SmolStr>) -> Self {
        self.doc = Some(doc.into());
        self
    }

    /// Returns the plan this rule is.
    #[must_use]
    pub const fn plan(&self) -> &Plan {
        &self.plan
    }

    /// Returns the specification's own wording of the mapping.
    #[must_use]
    pub fn doc(&self) -> Option<&str> {
        self.doc.as_deref()
    }

    /// Holds this rule to what the document can state.
    ///
    /// A rule fills columns, so its plan states a `select` naming at least
    /// one, and every projection names what it fills: a term with no name of
    /// its own would be a rule about a column nobody can find.
    fn validate(&self) -> Result<()> {
        self.plan
            .check_budget()
            .map_err(|error| refused(format_smolstr!("{error}")))?;
        let selector = self.plan.selector();
        if selector.is_all() || selector.is_empty() {
            return Err(refused(format_smolstr!(
                "expected every entry to fill at least one named column, {} names none",
                self.plan
            )));
        }
        Ok(())
    }

    /// Renders this rule into the document being written.
    fn write_into(&self, writer: &mut Writer) -> Result<()> {
        writer.open_element();
        writer.text(true, PLAN, &self.plan.to_string())?;
        if let Some(doc) = self.doc() {
            writer.text(false, DOC, doc)?;
        }
        writer.close_element();
        Ok(())
    }
}

/// A refusal the writer raises, before anything is written.
fn refused(reason: SmolStr) -> Error {
    Error::Parse {
        target: TARGET,
        position: 0,
        reason,
    }
}

/// One rule, borrowed from the stored document.
#[derive(Clone, Copy, Debug)]
pub struct FixReplacementEntry<'field> {
    plan: &'field str,
    doc: &'field str,
}

impl<'field> FixReplacementEntry<'field> {
    /// Returns the plan's canonical text, still escaped as stored.
    #[must_use]
    pub const fn plan(self) -> &'field str {
        self.plan
    }

    /// Reads the plan this rule is.
    ///
    /// # Errors
    ///
    /// Returns the JSON codec's refusal when the stored text is not a legal
    /// string body, the expression grammar's when it is not a plan, and
    /// [`Error::Parse`] when the plan is past the expression budget.
    pub fn parse_plan(self) -> Result<Plan> {
        let text = decode_text(TARGET, PLAN, Some(self.plan))?
            .ok_or_else(|| refused(format_smolstr!("expected {PLAN:?} to state a plan")))?;
        let plan: Plan = text
            .parse()
            .map_err(|error: Error| refused(format_smolstr!("{error}")))?;
        plan.check_budget()
            .map_err(|error| refused(format_smolstr!("{error}")))?;
        Ok(plan)
    }

    /// Returns the specification's wording, still escaped as stored.
    #[must_use]
    pub const fn doc(self) -> Option<&'field str> {
        if self.doc.is_empty() {
            None
        } else {
            Some(self.doc)
        }
    }

    /// Decodes the specification's wording.
    ///
    /// # Errors
    ///
    /// Returns the JSON codec's refusal when the stored text is not a legal
    /// string body, which the writer never produces.
    pub fn parse_doc(self) -> Result<Option<String>> {
        decode_text(TARGET, DOC, self.doc())
    }
}

/// Walks the rules one field carries, in document order.
pub struct FixReplacements<'field> {
    cursor: Cursor<'field>,
    started: bool,
    done: bool,
}

impl<'field> FixReplacements<'field> {
    /// Walks one stored `FIX:replacements` value, or nothing for an absent
    /// one.
    pub(super) fn over(stored: Option<&'field str>) -> Self {
        Self {
            cursor: Cursor::new(stored.unwrap_or_default()),
            started: false,
            done: stored.is_none(),
        }
    }

    /// Renders rules into the one canonical document they have.
    ///
    /// Order is kept, because it is what the document says: the first entry
    /// whose condition a message meets answers.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Parse`] when an entry's plan names no column to fill
    /// or is past the expression budget.
    pub(super) fn render(entries: &[FixReplacement]) -> Result<String> {
        entries.iter().try_for_each(FixReplacement::validate)?;
        let mut writer = Writer::open_array();
        for entry in entries {
            entry.write_into(&mut writer)?;
        }
        Ok(writer.finish())
    }

    /// Advances one step: the next entry, the document's end, or a refusal.
    fn step(&mut self) -> Scan<Option<FixReplacementEntry<'field>>> {
        if !self.cursor.next_entry(&mut self.started)? {
            return Ok(None);
        }
        self.read_entry().map(Some)
    }

    /// Reads the one entry starting at the cursor.
    fn read_entry(&mut self) -> Scan<FixReplacementEntry<'field>> {
        self.cursor.expect(b'{')?;
        let mut plan = None;
        let mut doc = "";
        let mut next = 0;
        loop {
            match KEYS[self.cursor.read_key(&KEYS, &mut next)?] {
                PLAN => plan = Some(self.cursor.read_string()?),
                _ => doc = self.cursor.read_string()?,
            }
            if !self.cursor.next_property()? {
                break;
            }
        }
        let Some(plan) = plan else {
            return Err(Refusal::MissingKey(PLAN));
        };
        Ok(FixReplacementEntry { plan, doc })
    }

    /// The next entry, answering nothing where the document does not parse.
    ///
    /// This is what every infallible reader walks, so a malformed document
    /// resolves to no answer rather than to a wrong one, and it spends no
    /// allocation doing it. The [`Iterator`] the same walk implements is the
    /// fallible door, naming the byte a hand edit stopped it at.
    pub fn next_ok(&mut self) -> Option<FixReplacementEntry<'field>> {
        self.next()?.ok()
    }
}

impl<'field> Iterator for FixReplacements<'field> {
    type Item = Result<FixReplacementEntry<'field>>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.done {
            return None;
        }
        match self.step() {
            Ok(None) => {
                self.done = true;
                None
            }
            Ok(Some(entry)) => Some(Ok(entry)),
            Err(refusal) => {
                self.done = true;
                Some(Err(refusal.into_error(
                    TARGET,
                    self.cursor.document(),
                    self.cursor.position(),
                )))
            }
        }
    }
}

impl FusedIterator for FixReplacements<'_> {}

impl From<FixReplacementEntry<'_>> for FixReplacement {
    /// The borrowed entry as an owned rule, for a caller editing one.
    ///
    /// A stored text the plan grammar refuses answers a rule whose plan is
    /// the empty one, which the writer then refuses rather than storing: a
    /// rule nobody can read is not a rule to write back.
    fn from(entry: FixReplacementEntry<'_>) -> Self {
        Self {
            plan: entry.parse_plan().unwrap_or_default(),
            doc: entry
                .parse_doc()
                .ok()
                .flatten()
                .map(SmolStr::new)
                .or_else(|| entry.doc().map(SmolStr::new)),
        }
    }
}
