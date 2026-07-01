//! Tests for the [`Whence`] seek origin.

use yggdryl_core::Whence;

#[test]
fn default_is_start_and_discriminants_are_stable() {
    assert_eq!(Whence::default(), Whence::Start);
    assert_eq!(Whence::Start as u8, 0);
    assert_eq!(Whence::Current as u8, 1);
    assert_eq!(Whence::End as u8, 2);
}

#[test]
fn is_hashable_so_it_can_key_a_map() {
    use std::collections::HashSet;
    let set: HashSet<Whence> = [Whence::Start, Whence::End, Whence::Start]
        .into_iter()
        .collect();
    assert_eq!(set.len(), 2);
}

#[cfg(feature = "serde")]
#[test]
fn round_trips_through_json_as_its_discriminant() {
    for whence in [Whence::Start, Whence::Current, Whence::End] {
        let json = serde_json::to_string(&whence).unwrap();
        assert_eq!(json, (whence as u8).to_string());
        assert_eq!(serde_json::from_str::<Whence>(&json).unwrap(), whence);
    }
    // An out-of-range discriminant is rejected with an actionable message.
    let err = serde_json::from_str::<Whence>("3").unwrap_err();
    assert!(err.to_string().contains("expected 0, 1 or 2"));
}
