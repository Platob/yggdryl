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
use std::sync::{Arc, Mutex, PoisonError};

use smol_str::SmolStr;

use super::FixId;
use super::registry::FixMap;
use crate::Field;
use crate::xxhash::Xxh64;

/// A table keyed by text, hashed by the crate's own function: a key is a
/// bridge's spelling or a wire value, read a million times per run.
type TextMap<K, V> = HashMap<K, V, Xxh64>;

/// What a run has learned, shared by every stream the codec is cloned into.
pub(super) struct Memo {
    /// The number this memo's answers are mirrored under: a memo dropped
    /// and another built at its address answer different dictionaries, a
    /// memo cleared answers a changed one, and a number is never given
    /// twice.
    id: AtomicU64,
    /// What one of the registry's own fields states about the values it
    /// takes, by the field's address in the registry.
    facts: Mutex<FixMap<usize, Arc<Facts>>>,
    /// The wire value a text spells for a field at a version.
    translations: Mutex<TextMap<Question, Option<SmolStr>>>,
    /// What the dictionary holds under one key.
    names: Mutex<TextMap<SmolStr, Lookup>>,
}

/// One translation asked: the field's address and the text.
type Question = (usize, SmolStr);

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

    /// The document of the set this field reads by, as the dictionary held it
    /// when the field was first asked about.
    pub(super) fn codes(&self) -> Option<&str> {
        self.codes.as_deref()
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

/// One thread's own copy of the answers it has read, by memo.
#[derive(Default)]
pub(super) struct Mirror {
    facts: FixMap<u64, FixMap<usize, Arc<Facts>>>,
    translations: FixMap<u64, TextMap<Question, Option<SmolStr>>>,
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

/// This thread's mirror exchanged with `with`: how a thread that dies with
/// its chunk hands what it has read to the one spawned for the next.
pub(super) fn swap_mirror(with: &mut Mirror) {
    MIRROR.with(|held| std::mem::swap(&mut *held.borrow_mut(), with));
}

impl Memo {
    /// How many answers each table remembers before it stops growing.
    pub(super) const CAPACITY: usize = 1 << 16;

    /// A dictionary that has answered nothing yet.
    pub(super) fn new() -> Self {
        Self {
            id: AtomicU64::new(MEMOS.fetch_add(1, Ordering::Relaxed)),
            facts: Mutex::new(FixMap::default()),
            translations: Mutex::new(HashMap::with_hasher(Xxh64::new())),
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
        held(&self.names).clear();
        self.id
            .store(MEMOS.fetch_add(1, Ordering::Relaxed), Ordering::Relaxed);
    }

    fn id(&self) -> u64 {
        self.id.load(Ordering::Relaxed)
    }

    /// What `source`, a field the registry keeps, states about its values,
    /// read off its metadata the first time and remembered.
    pub(super) fn facts(&self, source: &Field, codes: Option<&str>) -> Arc<Facts> {
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
                    codes: codes.map(Box::from),
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
    /// [`FixField::code_value`](crate::FixField::code_value) answers it, read
    /// once per distinct question. A field carrying no code set answers
    /// `None` without touching the table.
    pub(super) fn translation(&self, source: &Field, facts: &Facts, text: &str) -> Option<SmolStr> {
        let stored = facts.codes.as_deref()?;
        let key = (address(source), SmolStr::new(text));
        let id = self.id();
        let mirrored = MIRROR.with(|mirror| {
            mirror
                .borrow()
                .translations
                .get(&id)
                .and_then(|table| table.get(&key))
                .cloned()
        });
        if let Some(answer) = mirrored {
            return answer;
        }
        let known = held(&self.translations).get(&key).cloned();
        let answer = match known {
            Some(answer) => answer,
            None => {
                let answer = super::codes::translate(stored, text).map(SmolStr::new);
                let mut table = held(&self.translations);
                if table.len() < Self::CAPACITY {
                    table.insert(key.clone(), answer.clone());
                }
                answer
            }
        };
        MIRROR.with(|mirror| {
            let mut mirror = mirror.borrow_mut();
            let table = table_of(&mut mirror.translations, id, || {
                HashMap::with_hasher(Xxh64::new())
            });
            if table.len() < Self::CAPACITY {
                table.insert(key, answer.clone());
            }
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
