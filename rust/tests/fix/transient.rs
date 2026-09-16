//! Which fields say they carry from one message of an instrument's life to
//! the next.
//!
//! The flag is here; the lifecycle does not read it yet. See the handoff note
//! on `fix:transient` for what remains.

use std::sync::Arc;

use yggdryl::FixRegistry;

fn registry() -> Arc<FixRegistry> {
    super::committed_registry()
}

#[test]
fn a_field_carries_unless_it_says_it_is_settled_to_one_message() {
    let registry = registry();
    // Nothing says so, so it carries: that is the default and it is the safe
    // one, because failing to carry loses what the capture knew.
    for name in ["cficode", "isincode", "miccode", "symbolticker", "currency"] {
        let field = registry.field_by_name(name).expect(name);
        assert!(field.as_fix().is_transient().expect(name), "{name}");
    }
    // These are about the one message that carried them.
    for name in [
        "updatedat",
        "createdat",
        "snapshotat",
        "recordedat",
        "msghash",
        "msgphash",
        "state",
        "sourceurl",
        "nofixentries",
    ] {
        let field = registry.field_by_name(name).expect(name);
        assert!(!field.as_fix().is_transient().expect(name), "{name}");
    }
}

#[test]
fn the_flag_round_trips_and_only_false_is_written() {
    let mut field = yggdryl::DataType::utf8().nullable_field("venuething");
    assert!(field.as_fix().is_transient().unwrap(), "true by default");
    assert_eq!(field.as_metadata().get("fix:transient"), None);

    field.as_fix_mut().set_transient(false).unwrap();
    assert!(!field.as_fix().is_transient().unwrap());
    assert_eq!(field.as_metadata().get("fix:transient"), Some("false"));

    // True is the default, so saying it stores nothing and clears a false.
    field.as_fix_mut().set_transient(true).unwrap();
    assert!(field.as_fix().is_transient().unwrap());
    assert_eq!(field.as_metadata().get("fix:transient"), None);

    // Anything else is a located refusal rather than a quiet default.
    field
        .set_metadata([("fix:transient", "yes")])
        .expect("stored");
    let refused = field.as_fix().is_transient().unwrap_err();
    assert!(refused.to_string().contains("fix:transient"), "{refused}");
}
