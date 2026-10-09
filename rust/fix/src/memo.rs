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
//! Each table is keyed by what its answer is a fact of, and each is bounded
//! at [`Memo::CAPACITY`] entries past which an answer is still given and
//! simply not remembered - a dictionary is a bounded vocabulary, and a
//! capture spelling more distinct keys or values than that is spelling junk,
//! which the entries keep and the row types as it can.
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

use super::field::FixShape;
use super::registry::FixMap;
use super::{FixField, FixId};
use yggdryl::xxhash::Xxh64;
use yggdryl::{Field, Metadata};
use yggdryl_market::{IdKey, IdSource, IdType};

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
    /// What a party's role or source text reads as, by the set document it
    /// is read through and the slot it fills.
    parties: Mutex<PartyWords>,
    /// The key an entry no dictionary field maps is read under, by the
    /// identifier names its level declares.
    identifier_keys: Mutex<IdentifierKeys>,
}

/// Every party word read, by the address of the set's document - `0` where
/// the dictionary states no set - and the slot, then by the trimmed text.
///
/// Bounded per document and slot at [`Memo::CAPACITY`] texts: a role or a
/// source is a code set's handful of spellings.
type PartyWords = FixMap<(usize, PartySlot), TextMap<SmolStr, PartyWord>>;

/// Every key read off an unmapped entry, by the `FIX:identifiers` text its
/// level declares - empty for none - then by whether a dictionary field is
/// tagged by the key (`[untagged, tagged]`), then by the key's text.
///
/// Bounded at [`Memo::CAPACITY`] declarations, each at as many keys: the
/// declarations are the dictionary's, and a bridge spells a bounded set of
/// keys.
type IdentifierKeys = TextMap<SmolStr, [TextMap<SmolStr, Option<IdKey>>; 2]>;

/// What a party's role or source text is read for: a party's
/// `PartyRole(452)`, its `PartyIDSource(447)`, or an account's
/// `AcctIDSource(660)`.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum PartySlot {
    /// A party's `PartyRole(452)`.
    Role,
    /// A party's `PartyIDSource(447)`.
    PartySource,
    /// An account's `AcctIDSource(660)`.
    AccountSource,
}

impl PartySlot {
    /// What a bare wire code no set names is spelled under in this slot,
    /// `partyrole99`, so it never reads as a word of its own.
    pub(super) const fn prefix(self) -> &'static str {
        match self {
            Self::Role => "partyrole",
            Self::PartySource => "partyidsource",
            Self::AccountSource => "acctidsource",
        }
    }
}

/// What a party's role or source text reads as, the slot's own answer:
/// `None` where the word it reads is no type or no source, which refuses
/// the party.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PartyWord {
    /// The type a role reads as.
    Role(Option<IdType>),
    /// The source a party's or an account's source reads as.
    Source(Option<IdSource>),
}

/// Every translation a field has answered, by the field's address.
///
/// Bounded per field at [`Memo::CAPACITY`] texts: a code set is a handful
/// of spellings, and a field asked more distinct texts than that is being
/// asked about values that are not codes.
type Translations = FixMap<usize, TextMap<SmolStr, Option<SmolStr>>>;

/// What one field states about the values it takes, read off its metadata
/// once: the spellings it declares as an absence, its code set, and the
/// shape its declared FIX datatype gives its text.
pub struct Facts {
    nulls: Box<[SmolStr]>,
    codes: Option<Arc<str>>,
    shape: FixShape,
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

    /// The shape the field's declared FIX datatype gives its wire text,
    /// exactly as [`FixField::shape`](super::field) read it off the field
    /// once: what the FIX dispatch branches on for every value.
    pub(super) fn shape(&self) -> FixShape {
        self.shape
    }

    /// The metadata a child aliasing `source` - the field these facts are
    /// of - carries: `FIX:alias` naming it. Built once and shared, so a
    /// message building the child neither allocates the map nor changes the
    /// shape its column plan is found by.
    pub(super) fn alias_metadata(&self, source: &Field) -> Metadata {
        self.alias
            .get_or_init(|| {
                // A plain key and value: the one refusal a metadata entry
                // has is a shape no field name takes.
                Metadata::from_entries([(super::field::ALIAS_OF, source.name())])
                    .unwrap_or_else(|_| Metadata::new())
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
    parties: FixMap<u64, PartyWords>,
    identifier_keys: FixMap<u64, IdentifierKeys>,
}

/// How many memos one thread mirrors at once: past it the oldest is
/// forgotten, so a dictionary edited a thousand times leaves no thousand
/// tables behind.
const MIRRORED_MEMOS: usize = 8;

/// The table `id` mirrors into, the oldest memo forgotten where this
/// thread mirrors too many.
fn table_of<V>(tables: &mut FixMap<u64, V>, id: u64, fresh: impl FnOnce() -> V) -> &mut V {
    if !tables.contains_key(&id)
        && tables.len() >= MIRRORED_MEMOS
        && let Some(oldest) = tables.keys().min().copied()
    {
        tables.remove(&oldest);
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
            parties: Mutex::new(PartyWords::default()),
            identifier_keys: Mutex::new(HashMap::with_hasher(Xxh64::new())),
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
        held(&self.parties).clear();
        held(&self.identifier_keys).clear();
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
                let view = FixField::new(source);
                let facts = Arc::new(Facts {
                    nulls: view.nulls().map(SmolStr::new).collect(),
                    shape: view.shape(),
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
    /// answers it, else the first code whose name `reads` - the member a
    /// field's value door reads a code's name as, where the member's own
    /// name is no spelling the set holds - read once per distinct question.
    ///
    /// Keyed by the document's address: the registry owns every document
    /// for as long as this memo answers for it, and a set replaced clears
    /// the memo, so an address names one document under one number.
    pub(super) fn wire_value(
        &self,
        document: &str,
        name: &str,
        reads: impl Fn(&str) -> bool,
    ) -> Option<SmolStr> {
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
                let set = super::codes::FixCodeSet::new("", document);
                let answer = set
                    .code_by_name(name)
                    .or_else(|| set.codes().flatten().find(|code| reads(code.name())))
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

    /// What the trimmed `text` reads as in `slot`, read through the set
    /// `document` - where the dictionary states one - answered by `read`
    /// the first time and remembered.
    ///
    /// Keyed by the document's address, as [`Self::wire_value`] is.
    pub(super) fn party_word(
        &self,
        slot: PartySlot,
        document: Option<&str>,
        text: &str,
        read: impl FnOnce() -> PartyWord,
    ) -> PartyWord {
        let key = (
            document.map_or(0, |document| document.as_ptr() as usize),
            slot,
        );
        let id = self.id();
        let mirrored = MIRROR.with(|mirror| {
            mirror
                .borrow()
                .parties
                .get(&id)
                .and_then(|table| table.get(&key))
                .and_then(|table| table.get(text))
                .cloned()
        });
        if let Some(answer) = mirrored {
            return answer;
        }
        let known = held(&self.parties)
            .get(&key)
            .and_then(|table| table.get(text))
            .cloned();
        let answer = match known {
            Some(answer) => answer,
            None => {
                let answer = read();
                remember_word(held(&self.parties).entry(key).or_default(), text, &answer);
                answer
            }
        };
        MIRROR.with(|mirror| {
            let mut mirror = mirror.borrow_mut();
            let table = table_of(&mut mirror.parties, id, PartyWords::default);
            remember_word(table.entry(key).or_default(), text, &answer);
        });
        answer
    }

    /// The key an unmapped entry's `key` is read under against the
    /// `declared` identifier names of its level - where a dictionary field
    /// is `tagged` by it, those alone - answered by `read` the first time
    /// and remembered: `None` where it names no identifier.
    ///
    /// Keyed by the declaration's text rather than its address, so two
    /// components declaring the same names share their answers.
    pub(super) fn identifier_key(
        &self,
        declared: Option<&str>,
        tagged: bool,
        key: &str,
        read: impl FnOnce() -> Option<IdKey>,
    ) -> Option<IdKey> {
        let declared = declared.unwrap_or("");
        let slot = usize::from(tagged);
        let id = self.id();
        let mirrored = MIRROR.with(|mirror| {
            mirror
                .borrow()
                .identifier_keys
                .get(&id)
                .and_then(|table| table.get(declared))
                .and_then(|tables| tables[slot].get(key))
                .cloned()
        });
        if let Some(answer) = mirrored {
            return answer;
        }
        let known = held(&self.identifier_keys)
            .get(declared)
            .and_then(|tables| tables[slot].get(key))
            .cloned();
        let answer = match known {
            Some(answer) => answer,
            None => {
                let answer = read();
                remember_key(
                    &mut held(&self.identifier_keys),
                    declared,
                    slot,
                    key,
                    &answer,
                );
                answer
            }
        };
        MIRROR.with(|mirror| {
            let mut mirror = mirror.borrow_mut();
            let table = table_of(&mut mirror.identifier_keys, id, || {
                HashMap::with_hasher(Xxh64::new())
            });
            remember_key(table, declared, slot, key, &answer);
        });
        answer
    }
}

/// Remembers one party word under its text, while the table has room.
fn remember_word(table: &mut TextMap<SmolStr, PartyWord>, text: &str, answer: &PartyWord) {
    if table.len() < Memo::CAPACITY {
        table.insert(SmolStr::new(text), answer.clone());
    }
}

/// Remembers one key's reading under its declaration and its text, while
/// both tables have room.
fn remember_key(
    tables: &mut IdentifierKeys,
    declared: &str,
    slot: usize,
    key: &str,
    answer: &Option<IdKey>,
) {
    if !tables.contains_key(declared) {
        if tables.len() >= Memo::CAPACITY {
            return;
        }
        let fresh = || HashMap::with_hasher(Xxh64::new());
        tables.insert(SmolStr::new(declared), [fresh(), fresh()]);
    }
    if let Some(tables) = tables.get_mut(declared)
        && tables[slot].len() < Memo::CAPACITY
    {
        tables[slot].insert(SmolStr::new(key), answer.clone());
    }
}

#[cfg(feature = "internals")]
#[doc(hidden)]
pub mod internals {
    //! What `rust/fix/tests/root/memo.rs` pins and a caller cannot reach.
    //!
    //! The memo is invisible except through the work it does *not* repeat, so
    //! the pin counts the calls a supplier is asked for rather than reading an
    //! answer. `fix::memo` is a private module of a published one, so these
    //! being `pub` reaches nobody: this door is the only path to them, and it
    //! exists under the `internals` feature alone.
    use std::sync::Arc;

    use yggdryl::Field;
    use yggdryl_market::IdKey;

    pub use super::{Facts, Memo, PartySlot, PartyWord};

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

    /// What the trimmed `text` reads as in `slot` through the set
    /// `document`, `read` answering the first time.
    pub fn party_word(
        memo: &Memo,
        slot: PartySlot,
        document: Option<&str>,
        text: &str,
        read: impl FnOnce() -> PartyWord,
    ) -> PartyWord {
        memo.party_word(slot, document, text, read)
    }

    /// The key an unmapped entry's `key` is read under against `declared`,
    /// `read` answering the first time.
    pub fn identifier_key(
        memo: &Memo,
        declared: Option<&str>,
        tagged: bool,
        key: &str,
        read: impl FnOnce() -> Option<IdKey>,
    ) -> Option<IdKey> {
        memo.identifier_key(declared, tagged, key, read)
    }

    /// The code document these facts resolved to, as the shared handle the
    /// memo kept, so a second ask can be shown to be the same one.
    #[must_use]
    pub fn codes(facts: &Facts) -> Option<&Arc<str>> {
        facts.codes.as_ref()
    }
}
