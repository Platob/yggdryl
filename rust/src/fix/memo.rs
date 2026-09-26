//! What a dictionary has already answered about itself.
//!
//! A codec reads a capture of millions of lines against one registry, and
//! most of what a line asks the dictionary is a question the line before it
//! asked: what the field `#AVEXDESTINATION` names (nothing), which spellings
//! `LastQty` states as an absence (none), what `2` translates to in
//! `ExecType` at FIX 4.2. Each answer is a fact of the registry and the text
//! alone, so the registry keeps the table, every codec and every row read
//! against it shares the answers, and a change to the dictionary forgets
//! them.
//!
//! Three tables, each keyed by what its answer is a fact of, each bounded at
//! [`Memo::CAPACITY`] entries past which an answer is still given and simply
//! not remembered - a dictionary is a bounded vocabulary, and a capture
//! spelling more distinct keys or values than that is spelling junk, which
//! the entries keep and the row types as it can.
//!
//! The tables are shared by every thread the codec reads on, and each
//! thread keeps its own mirror of the answers it has read: a line asks its
//! questions off the mirror without a lock, and only a question the mirror
//! has not seen reaches the shared table - so four threads reading one
//! capture contend on nothing a line asks twice.

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};

use smol_str::SmolStr;

use super::FixId;
use super::registry::FixMap;
use crate::xxhash::Xxh64;
use crate::{Field, Metadata};

/// A table keyed by text, hashed by the crate's own function: a key is a
/// bridge's spelling or a wire value, read a million times per run.
type TextMap<K, V> = HashMap<K, V, Xxh64>;

/// What a run has learned, shared by every stream the codec is cloned into.
pub struct Memo {
    /// The number this memo's answers are mirrored under: a memo dropped
    /// and another built at its address answer different dictionaries, a
    /// memo cleared answers a changed one, and a number is never given
    /// twice.
    id: AtomicU64,
    /// What one of the registry's own fields states about the values it
    /// takes, by the field's address in the registry.
    facts: Mutex<FixMap<usize, Arc<Facts>>>,
    /// The wire value a text spells for a field: by the field's address,
    /// then by the text, so a question is asked with the text borrowed and
    /// owns it only where the answer is first remembered.
    translations: Mutex<Translations>,
    /// The wire value a code's name spells, by the address of the set's
    /// document: the reverse of `translations`, which a message asks for
    /// every coded value it re-emits.
    wire_values: Mutex<Translations>,
    /// What the dictionary holds under one key.
    names: Mutex<TextMap<SmolStr, Lookup>>,
}

/// Every translation a field has answered, by the field's address.
///
/// Bounded per field at [`Memo::CAPACITY`] texts: a code set is a handful
/// of spellings, and a field asked more distinct texts than that is being
/// asked about values that are not codes.
type Translations = FixMap<usize, TextMap<SmolStr, Option<SmolStr>>>;

/// What one field states about the values it takes, read off its metadata
/// once: the spellings it declares as an absence, and its code set.
pub struct Facts {
    nulls: Box<[SmolStr]>,
    codes: Option<Arc<str>>,
    /// The metadata a child aliasing the field carries, built the first
    /// time one is and shared after: one storage, so every message that
    /// builds such a child has the shape the first one had.
    alias: OnceLock<Metadata>,
}

impl Facts {
    /// Whether `text` is a spelling this field states as an absence, through
    /// the predicate [`FixField::is_null_value`](crate::FixField::is_null_value)
    /// answers with, over the spellings read off the field once.
    pub(super) fn is_null(&self, text: &str) -> bool {
        super::field::spells_absence(self.nulls.iter().map(SmolStr::as_str), text)
    }

    /// The document of the set this field reads by, as the dictionary held it
    /// when the field was first asked about.
    pub(super) fn codes(&self) -> Option<&str> {
        self.codes.as_deref()
    }

    /// The metadata a child aliasing `source` - the field these facts are
    /// of - carries: `FIX:alias` naming it. Built once and shared, so a
    /// message building the child neither allocates the map nor changes the
    /// shape its column plan is found by.
    pub(super) fn alias_metadata(&self, source: &Field) -> Metadata {
        self.alias
            .get_or_init(|| {
                let mut metadata = Metadata::new();
                // A plain key and value: the one refusal a metadata insert
                // has is a shape no field name takes.
                let _ =
                    metadata.insert(super::field::ALIAS_OF.to_owned(), source.name().to_owned());
                metadata
            })
            .clone()
    }
}

/// What the dictionary holds under one key: the field the key names, by its
/// tag and identity, and whether the key names a repeating group.
#[derive(Clone, Copy)]
pub(super) struct Lookup {
    pub(super) field: Option<(i32, FixId)>,
    pub(super) group: bool,
    /// A dotted root name's last segment, resolved once before its value is
    /// typed. Dotted path lookup itself remains schema-scoped in the builder.
    pub(super) composed: Option<(i32, FixId)>,
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

/// How many memos have been numbered, which numbers the next.
static MEMOS: AtomicU64 = AtomicU64::new(1);

/// Remembers one translation under its text, while the field's table has
/// room.
fn remember(table: &mut TextMap<SmolStr, Option<SmolStr>>, text: &str, answer: &Option<SmolStr>) {
    if table.len() < Memo::CAPACITY {
        table.insert(SmolStr::new(text), answer.clone());
    }
}

/// One thread's own copy of the answers it has read, by memo.
#[derive(Default)]
struct Mirror {
    facts: FixMap<u64, FixMap<usize, Arc<Facts>>>,
    translations: FixMap<u64, Translations>,
    wire_values: FixMap<u64, Translations>,
    names: FixMap<u64, TextMap<SmolStr, Lookup>>,
}

/// How many memos one thread mirrors at once: past it the oldest is
/// forgotten, so a dictionary edited a thousand times leaves no thousand
/// tables behind.
const MIRRORED_MEMOS: usize = 8;

/// The table `id` mirrors into, the oldest memo forgotten where this
/// thread mirrors too many.
fn table_of<V>(tables: &mut FixMap<u64, V>, id: u64, fresh: impl FnOnce() -> V) -> &mut V {
    if !tables.contains_key(&id) && tables.len() >= MIRRORED_MEMOS {
        if let Some(oldest) = tables.keys().min().copied() {
            tables.remove(&oldest);
        }
    }
    tables.entry(id).or_insert_with(fresh)
}

thread_local! {
    static MIRROR: RefCell<Mirror> = RefCell::new(Mirror::default());
}

impl Memo {
    /// How many answers each table remembers before it stops growing.
    pub(super) const CAPACITY: usize = 1 << 16;

    /// A dictionary that has answered nothing yet.
    pub(super) fn new() -> Self {
        Self {
            id: AtomicU64::new(MEMOS.fetch_add(1, Ordering::Relaxed)),
            facts: Mutex::new(FixMap::default()),
            translations: Mutex::new(Translations::default()),
            wire_values: Mutex::new(Translations::default()),
            names: Mutex::new(HashMap::with_hasher(Xxh64::new())),
        }
    }

    /// Forgets every answer: what the dictionary answers has changed.
    ///
    /// The number moves with them, so what a thread mirrored under the old
    /// one is never read again.
    pub(super) fn clear(&self) {
        held(&self.facts).clear();
        held(&self.translations).clear();
        held(&self.wire_values).clear();
        held(&self.names).clear();
        self.id
            .store(MEMOS.fetch_add(1, Ordering::Relaxed), Ordering::Relaxed);
    }

    fn id(&self) -> u64 {
        self.id.load(Ordering::Relaxed)
    }

    /// What `source`, a field the registry keeps, states about its values,
    /// read off its metadata the first time and remembered.
    pub(super) fn facts(
        &self,
        source: &Field,
        codes: impl FnOnce() -> Option<Arc<str>>,
    ) -> Arc<Facts> {
        let key = address(source);
        let id = self.id();
        let mirrored = MIRROR.with(|mirror| {
            mirror
                .borrow()
                .facts
                .get(&id)
                .and_then(|table| table.get(&key))
                .map(Arc::clone)
        });
        if let Some(facts) = mirrored {
            return facts;
        }
        // The shared table's answer is read and the lock released before
        // the answer is made, so a miss never takes the lock twice.
        let known = held(&self.facts).get(&key).map(Arc::clone);
        let facts = match known {
            Some(facts) => facts,
            None => {
                let view = source.as_fix();
                let facts = Arc::new(Facts {
                    nulls: view.nulls().map(SmolStr::new).collect(),
                    // The set the field names, resolved once here rather than
                    // once per entry: a run translates a million spellings
                    // through one document and looks its name up once.
                    codes: codes(),
                    alias: OnceLock::new(),
                });
                let mut table = held(&self.facts);
                if table.len() < Self::CAPACITY {
                    table.insert(key, Arc::clone(&facts));
                }
                facts
            }
        };
        MIRROR.with(|mirror| {
            let mut mirror = mirror.borrow_mut();
            let table = table_of(&mut mirror.facts, id, FixMap::default);
            if table.len() < Self::CAPACITY {
                table.insert(key, Arc::clone(&facts));
            }
        });
        facts
    }

    /// The wire value `text` spells for `source`, exactly as
    /// [`FixCodeSet::code_value`](super::FixCodeSet::code_value) answers it, read
    /// once per distinct question. A field carrying no code set answers
    /// `None` without touching the table.
    pub(super) fn translation(&self, source: &Field, facts: &Facts, text: &str) -> Option<SmolStr> {
        let stored = facts.codes.as_deref()?;
        let key = address(source);
        let id = self.id();
        // Asked with the text borrowed: a hit on the mirror, which is what
        // every value after the first of its spelling is, owns nothing.
        let mirrored = MIRROR.with(|mirror| {
            mirror
                .borrow()
                .translations
                .get(&id)
                .and_then(|table| table.get(&key))
                .and_then(|table| table.get(text))
                .cloned()
        });
        if let Some(answer) = mirrored {
            return answer;
        }
        let known = held(&self.translations)
            .get(&key)
            .and_then(|table| table.get(text))
            .cloned();
        let answer = match known {
            Some(answer) => answer,
            None => {
                let answer = super::codes::translate(stored, text).map(SmolStr::new);
                let mut table = held(&self.translations);
                remember(table.entry(key).or_default(), text, &answer);
                answer
            }
        };
        MIRROR.with(|mirror| {
            let mut mirror = mirror.borrow_mut();
            let table = table_of(&mut mirror.translations, id, Translations::default);
            remember(table.entry(key).or_default(), text, &answer);
        });
        answer
    }

    /// The wire value the code named `name` has in the set `document` is,
    /// exactly as [`FixCodeSet::code_by_name`](super::FixCodeSet::code_by_name)
    /// answers it, read once per distinct question.
    ///
    /// Keyed by the document's address: the registry owns every document
    /// for as long as this memo answers for it, and a set replaced clears
    /// the memo, so an address names one document under one number.
    pub(super) fn wire_value(&self, document: &str, name: &str) -> Option<SmolStr> {
        let key = document.as_ptr() as usize;
        let id = self.id();
        let mirrored = MIRROR.with(|mirror| {
            mirror
                .borrow()
                .wire_values
                .get(&id)
                .and_then(|table| table.get(&key))
                .and_then(|table| table.get(name))
                .cloned()
        });
        if let Some(answer) = mirrored {
            return answer;
        }
        let known = held(&self.wire_values)
            .get(&key)
            .and_then(|table| table.get(name))
            .cloned();
        let answer = match known {
            Some(answer) => answer,
            None => {
                let answer = super::codes::FixCodeSet::new("", document)
                    .code_by_name(name)
                    .map(|code| SmolStr::new(code.value()));
                let mut table = held(&self.wire_values);
                remember(table.entry(key).or_default(), name, &answer);
                answer
            }
        };
        MIRROR.with(|mirror| {
            let mut mirror = mirror.borrow_mut();
            let table = table_of(&mut mirror.wire_values, id, Translations::default);
            remember(table.entry(key).or_default(), name, &answer);
        });
        answer
    }

    /// What the dictionary holds under `key`, answered by `ask` the first
    /// time and remembered.
    pub(super) fn lookup(&self, key: &str, ask: impl FnOnce() -> Lookup) -> Lookup {
        let id = self.id();
        let mirrored = MIRROR.with(|mirror| {
            mirror
                .borrow()
                .names
                .get(&id)
                .and_then(|table| table.get(key))
                .copied()
        });
        if let Some(answer) = mirrored {
            return answer;
        }
        let known = held(&self.names).get(key).copied();
        let answer = match known {
            Some(answer) => answer,
            None => {
                let answer = ask();
                let mut table = held(&self.names);
                if table.len() < Self::CAPACITY {
                    table.insert(SmolStr::new(key), answer);
                }
                answer
            }
        };
        MIRROR.with(|mirror| {
            let mut mirror = mirror.borrow_mut();
            let table = table_of(&mut mirror.names, id, || HashMap::with_hasher(Xxh64::new()));
            if table.len() < Self::CAPACITY {
                table.insert(SmolStr::new(key), answer);
            }
        });
        answer
    }
}

#[cfg(feature = "internals")]
#[doc(hidden)]
pub mod internals {
    //! What `rust/tests/fix/memo.rs` pins and a caller cannot reach.
    //!
    //! The memo is invisible except through the work it does *not* repeat, so
    //! the pin counts the calls a supplier is asked for rather than reading an
    //! answer. `fix::memo` is a private module of a published one, so these
    //! being `pub` reaches nobody: this door is the only path to them, and it
    //! exists under the `internals` feature alone.
    use std::sync::Arc;

    use crate::Field;

    pub use super::{Facts, Memo};

    /// A dictionary that has answered nothing yet.
    #[must_use]
    pub fn new() -> Memo {
        Memo::new()
    }

    /// Forget every answer, as a changed dictionary does.
    pub fn clear(memo: &Memo) {
        memo.clear();
    }

    /// What `source` states about its values, read off its metadata the first
    /// time and remembered, `codes` supplying the document that first time.
    pub fn facts(
        memo: &Memo,
        source: &Field,
        codes: impl FnOnce() -> Option<Arc<str>>,
    ) -> Arc<Facts> {
        memo.facts(source, codes)
    }

    /// The code document these facts resolved to, as the shared handle the
    /// memo kept, so a second ask can be shown to be the same one.
    #[must_use]
    pub fn codes(facts: &Facts) -> Option<&Arc<str>> {
        facts.codes.as_ref()
    }
}
