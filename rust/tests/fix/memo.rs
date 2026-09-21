//! `rust/src/fix/memo.rs`: what a dictionary remembers about its own fields.
//!
//! The memo is invisible except through the work it does *not* repeat, so this
//! counts the calls the supplier is asked for. Nothing a caller holds names it,
//! so it is reached through `yggdryl::internals`.

use std::cell::Cell;
use std::sync::Arc;

use yggdryl::DataType;
use yggdryl::internals::fix_memo::{clear, codes, facts, new};

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
