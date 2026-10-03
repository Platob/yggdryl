//! `rust/src/fix/memo.rs`: what a dictionary remembers about its own fields.
//!
//! The memo is invisible except through the work it does *not* repeat, so this
//! counts the calls the supplier is asked for. Nothing a caller holds names it,
//! so it is reached through `yggdryl::internals`.

use std::cell::Cell;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use yggdryl::internals::fix_memo::{
    PartySlot, PartyWord, clear, codes, facts, identifier_key, new, party_word,
};
use yggdryl::{DataType, IdKey, IdSource, IdType};

#[test]
fn field_facts_resolve_and_share_the_code_document_once_until_cleared() {
    let memo = new();
    let field = DataType::utf8().nullable_field("side");
    let calls = Cell::new(0);
    let first: Arc<str> = Arc::from(r#"[{"value":"1","name":"Buy"}]"#);
    let supplied = || {
        calls.set(calls.get() + 1);
        Some(Arc::clone(&first))
    };

    let resolved = facts(&memo, &field, supplied);
    let repeated = facts(&memo, &field, supplied);
    assert_eq!(calls.get(), 1);
    assert!(Arc::ptr_eq(codes(&resolved).unwrap(), &first));
    assert!(Arc::ptr_eq(&resolved, &repeated));

    clear(&memo);
    let replacement: Arc<str> = Arc::from(r#"[{"value":"2","name":"Sell"}]"#);
    let refreshed = facts(&memo, &field, || {
        calls.set(calls.get() + 1);
        Some(Arc::clone(&replacement))
    });
    assert_eq!(calls.get(), 2);
    assert!(Arc::ptr_eq(codes(&refreshed).unwrap(), &replacement));
    assert!(!Arc::ptr_eq(&resolved, &refreshed));
}

#[test]
fn a_party_word_is_read_once_per_set_slot_and_text_until_cleared() {
    let memo = new();
    let calls = AtomicUsize::new(0);
    // Two documents spelling the same set: the memo keys a document by where
    // the registry holds it, so each is asked for on its own.
    let set = String::from(r#"[{"value":"1","name":"ExecutingFirm"}]"#);
    let other = set.clone();
    let read = |word: PartyWord| {
        let calls = &calls;
        move || {
            calls.fetch_add(1, Ordering::Relaxed);
            word
        }
    };
    let firm = PartyWord::Role(Some(IdType::ExecutingFirm));
    let refused = PartyWord::Role(None);

    let asked = party_word(&memo, PartySlot::Role, Some(&set), "1", read(firm.clone()));
    assert_eq!(asked, firm);
    // Remembered on this thread's mirror, and in the table every thread
    // shares: neither asks again, so the answer `read` would give is unread.
    let mirrored = party_word(
        &memo,
        PartySlot::Role,
        Some(&set),
        "1",
        read(refused.clone()),
    );
    let shared = std::thread::scope(|scope| {
        scope
            .spawn(|| {
                party_word(
                    &memo,
                    PartySlot::Role,
                    Some(&set),
                    "1",
                    read(refused.clone()),
                )
            })
            .join()
            .unwrap()
    });
    assert_eq!((mirrored, shared), (firm.clone(), firm.clone()));
    assert_eq!(calls.load(Ordering::Relaxed), 1);

    // Another slot, another document, no set and another text are each
    // questions of their own, a refusal remembered as an answer.
    let source = PartyWord::Source(Some(IdSource::Base));
    let slot = party_word(
        &memo,
        PartySlot::PartySource,
        Some(&set),
        "1",
        read(source.clone()),
    );
    let document = party_word(
        &memo,
        PartySlot::Role,
        Some(&other),
        "1",
        read(refused.clone()),
    );
    let unset = party_word(&memo, PartySlot::Role, None, "1", read(refused.clone()));
    let text = party_word(
        &memo,
        PartySlot::Role,
        Some(&set),
        "2",
        read(refused.clone()),
    );
    assert_eq!(
        (slot, document, unset, text),
        (source, refused.clone(), refused.clone(), refused.clone())
    );
    assert_eq!(calls.load(Ordering::Relaxed), 5);
    let again = party_word(&memo, PartySlot::Role, None, "1", read(firm.clone()));
    assert_eq!(again, refused);
    assert_eq!(calls.load(Ordering::Relaxed), 5);

    // A changed dictionary forgets them all.
    clear(&memo);
    let clearing = PartyWord::Role(Some(IdType::ClearingFirm));
    let refreshed = party_word(
        &memo,
        PartySlot::Role,
        Some(&set),
        "1",
        read(clearing.clone()),
    );
    assert_eq!(refreshed, clearing);
    assert_eq!(calls.load(Ordering::Relaxed), 6);
}

#[test]
fn an_unmapped_key_is_read_once_per_declaration_and_key_until_cleared() {
    let memo = new();
    let calls = AtomicUsize::new(0);
    let read = |answer: Option<IdKey>| {
        let calls = &calls;
        move || {
            calls.fetch_add(1, Ordering::Relaxed);
            answer
        }
    };
    let order = Some(IdKey::new(IdSource::Base, IdType::OrderId));

    let asked = identifier_key(&memo, None, false, "OMS_OrderID", read(order.clone()));
    assert_eq!(asked, order);
    // No declaration and an empty one declare the same names.
    let mirrored = identifier_key(&memo, Some(""), false, "OMS_OrderID", read(None));
    let shared = std::thread::scope(|scope| {
        scope
            .spawn(|| identifier_key(&memo, None, false, "OMS_OrderID", read(None)))
            .join()
            .unwrap()
    });
    assert_eq!((mirrored, shared), (order.clone(), order.clone()));
    assert_eq!(calls.load(Ordering::Relaxed), 1);

    // A tagged key, another declaration and another key are each asked; a
    // key naming nothing is remembered as naming nothing.
    let tagged = identifier_key(&memo, None, true, "OMS_OrderID", read(None));
    let declared = identifier_key(&memo, Some("reforderid"), false, "OMS_OrderID", read(None));
    let other = identifier_key(&memo, None, false, "OMS_Venue", read(None));
    assert_eq!((tagged, declared, other), (None, None, None));
    assert_eq!(calls.load(Ordering::Relaxed), 4);
    assert_eq!(
        identifier_key(&memo, None, false, "OMS_Venue", read(order.clone())),
        None
    );
    assert_eq!(calls.load(Ordering::Relaxed), 4);

    clear(&memo);
    assert_eq!(
        identifier_key(&memo, None, false, "OMS_OrderID", read(None)),
        None
    );
    assert_eq!(calls.load(Ordering::Relaxed), 5);
}
