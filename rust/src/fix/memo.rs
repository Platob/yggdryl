//! What one run has already answered about its dictionary.
//!
//! A codec reads a capture of millions of lines against one registry, and
//! most of what a line asks the dictionary is a question the line before it
//! asked: what the field `#AVEXDESTINATION` names (nothing), which spellings
//! `LastQty` states as an absence (none), what `2` translates to in
//! `ExecType` at FIX 4.2. Each answer is a fact of the registry, the branch,
//! and the text alone, and the registry sits behind an `Arc` the codec holds
//! for as long as this table lives, so an answer read once is an answer.
//!
//! Three tables, each keyed by what its answer is a fact of, each bounded at
//! [`Memo::CAPACITY`] entries past which an answer is still given and simply
//! not remembered - a dictionary is a bounded vocabulary, and a capture
//! spelling more distinct keys or values than that is spelling junk, which
//! the entries keep and the row types as it can.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};

use smol_str::SmolStr;

use super::{FixBranch, FixId};
use crate::{Field, Version};

/// What a run has learned, shared by every stream the codec is cloned into.
pub(super) struct Memo {
    /// What one of the registry's own fields states about the values it
    /// takes, by the field's address in the registry.
    facts: Mutex<HashMap<usize, Arc<Facts>>>,
    /// The wire value a text spells for a field at a version.
    translations: Mutex<HashMap<Question, Option<SmolStr>>>,
    /// What the dictionary holds under one key in one branch tier.
    names: Mutex<HashMap<(u32, SmolStr), Lookup>>,
}

/// One translation asked: the field's address, the version the read is
/// pinned to, the text.
type Question = (usize, Option<Version>, SmolStr);

/// What one field states about the values it takes, read off its metadata
/// once: the spellings it declares as an absence, and its code set.
pub(super) struct Facts {
    nulls: Box<[SmolStr]>,
    codes: Option<Box<str>>,
}

impl Facts {
    /// Whether `text` is a spelling this field states as an absence, through
    /// the predicate [`FixField::is_null_value`](crate::FixField::is_null_value)
    /// answers with, over the spellings read off the field once.
    pub(super) fn is_null(&self, text: &str) -> bool {
        super::field::spells_absence(self.nulls.iter().map(SmolStr::as_str), text)
    }
}

/// What the dictionary holds under one key: the field the key names, by its
/// identity, and whether the key names a repeating group.
#[derive(Clone, Copy)]
pub(super) struct Lookup {
    pub(super) field: Option<FixId>,
    pub(super) group: bool,
}

/// One table's lock, poisoned or not: a table holds answers, and a panic
/// mid-insert leaves it holding answers.
fn held<T>(table: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    table.lock().unwrap_or_else(PoisonError::into_inner)
}

/// One field's address, which names it for as long as the registry lives.
fn address(field: &Field) -> usize {
    std::ptr::from_ref(field) as usize
}

impl Memo {
    /// How many answers each table remembers before it stops growing.
    pub(super) const CAPACITY: usize = 1 << 16;

    /// A run that has answered nothing yet.
    pub(super) fn new() -> Self {
        Self {
            facts: Mutex::new(HashMap::new()),
            translations: Mutex::new(HashMap::new()),
            names: Mutex::new(HashMap::new()),
        }
    }

    /// What `source`, one of the registry's own fields, states about its
    /// values, read off its metadata the first time and remembered.
    pub(super) fn facts(&self, source: &Field) -> Arc<Facts> {
        let key = address(source);
        if let Some(facts) = held(&self.facts).get(&key) {
            return Arc::clone(facts);
        }
        let view = source.as_fix();
        let facts = Arc::new(Facts {
            nulls: view.nulls().map(SmolStr::new).collect(),
            codes: view.codes_document().map(Box::from),
        });
        let mut table = held(&self.facts);
        if table.len() < Self::CAPACITY {
            table.insert(key, Arc::clone(&facts));
        }
        facts
    }

    /// The wire value `text` spells for `source` at `at`, exactly as
    /// [`FixField::code_value_at`](crate::FixField::code_value_at) answers
    /// it - and as [`FixField::code_value`](crate::FixField::code_value)
    /// does where `at` is `None` - read once per distinct question. A field
    /// carrying no code set answers `None` without touching the table.
    pub(super) fn translation(
        &self,
        source: &Field,
        facts: &Facts,
        text: &str,
        at: Option<Version>,
    ) -> Option<SmolStr> {
        let stored = facts.codes.as_deref()?;
        let key = (address(source), at, SmolStr::new(text));
        if let Some(answer) = held(&self.translations).get(&key) {
            return answer.clone();
        }
        let answer = super::field::translate(stored, text, at).map(SmolStr::new);
        let mut table = held(&self.translations);
        if table.len() < Self::CAPACITY {
            table.insert(key, answer.clone());
        }
        answer
    }

    /// What the dictionary holds under `key` in `branch`'s tier, answered by
    /// `ask` the first time and remembered.
    pub(super) fn lookup(
        &self,
        branch: &FixBranch,
        key: &str,
        ask: impl FnOnce() -> Lookup,
    ) -> Lookup {
        let question = (branch.digest(), SmolStr::new(key));
        if let Some(answer) = held(&self.names).get(&question) {
            return *answer;
        }
        let answer = ask();
        let mut table = held(&self.names);
        if table.len() < Self::CAPACITY {
            table.insert(question, answer);
        }
        answer
    }
}
