//! `rust/src/fix/msgtype.rs`: a registry's message definition read by its
//! wire code, its business category typed as the member it names.

use yggdryl::MarketDataKind;

use super::{committed_registry, msgtypes};

/// Every message definition of the committed dictionary answers its
/// `FIX:msgcat` as the member of `MarketDataKind` it names - the enum is the
/// one owner of the set, so a category it does not name would have refused
/// the dictionary at load - and none where it files none.
#[test]
fn every_message_answers_its_category_as_the_member_it_names() {
    let registry = committed_registry();
    let mut filed = 0;
    for field in msgtypes(&registry) {
        let code = field
            .as_fix()
            .msgtype()
            .expect("a message names a wire code");
        let msgtype = registry
            .get_msgtype(code)
            .expect("a registered wire code reads back");
        match field.as_fix().msgcat() {
            Some(text) => {
                let member = msgtype.marketdatakind();
                assert!(member.is_some(), "{code}: {text}");
                assert_eq!(member.map(MarketDataKind::as_str), Some(text), "{code}");
                filed += 1;
            }
            None => assert_eq!(msgtype.marketdatakind(), None, "{code}"),
        }
    }
    assert!(filed > 100, "the dictionary files its messages: {filed}");
    assert_eq!(
        registry
            .get_msgtype("D")
            .and_then(|held| held.marketdatakind()),
        Some(MarketDataKind::Order)
    );
    assert_eq!(
        registry
            .get_msgtype("W")
            .and_then(|held| held.marketdatakind()),
        Some(MarketDataKind::Book)
    );
}
