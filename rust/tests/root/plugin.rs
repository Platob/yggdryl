//! `rust/src/plugin.rs`: the claim-once register every extension point
//! lands on - a key claimed exactly once, read any number of times, and a
//! second claim refused naming the first claimant.

use yggdryl::plugin::Register;

static WORDS: Register<&'static str, u8> = Register::new("word");

#[test]
fn a_key_is_claimed_once_and_a_second_claim_names_the_first_claimant() {
    assert!(WORDS.is_empty());
    WORDS.claim("alpha", 1, "first-crate").unwrap();
    WORDS.claim("beta", 2, "first-crate").unwrap();
    assert_eq!(WORDS.get("alpha"), Some(1));
    assert_eq!(WORDS.get("gamma"), None);
    assert_eq!(WORDS.claimant("beta"), Some("first-crate"));
    assert_eq!(WORDS.claimant("gamma"), None);
    assert_eq!(WORDS.values(), [1, 2]);
    assert_eq!(WORDS.len(), 2);
    // Claimed by another crate, or by the same one again: refused, the
    // first claimant and the key named, the first claim standing.
    for by in ["second-crate", "first-crate"] {
        let refused = WORDS.claim("alpha", 9, by).unwrap_err();
        assert!(refused.is_conflict());
        let text = refused.to_string();
        assert!(text.contains("first-crate"), "{text}");
        assert!(text.contains("alpha"), "{text}");
        assert!(text.contains("word"), "{text}");
    }
    assert_eq!(WORDS.get("alpha"), Some(1));
    assert_eq!(WORDS.len(), 2);
}
