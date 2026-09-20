//! Party identifier source codes, shared by every Party family member.

use super::{SoleMessage, committed_registry, fixed_codec, path};
use yggdryl::{FixCodec, Scalar};

fn reader() -> FixCodec {
    fixed_codec(committed_registry()).with_exclude_msgtypes::<[&str; 0], &str>([])
}

#[test]
fn party_source_family_resolves_the_bridge_proprietary_alias_to_d() {
    let registry = committed_registry();
    for tag in [447, 525] {
        let field = registry.field_by_tag(tag).expect("a PartyIDSource field");
        let codes = registry
            .codeset_of(field)
            .expect("the PartyIDSource code set");
        assert_eq!(codes.code_value("proprietary/customcode"), Some("D"));
        assert_eq!(
            codes.code("D").expect("the canonical code").name(),
            "Proprietary"
        );
    }
}

#[test]
fn a_nested_bridge_party_source_is_typed_to_d_and_writes_the_wire_value() {
    let reader = reader();
    let parsed = reader
        .sole_line(
            b"MSGTYPE=D|#NOPARTYIDS=1|#NOPARTYIDS[0]=PARTYID=SYNTH-01\x04\x03PARTYIDSOURCE=proprietary/customcode\x04\x03PARTYROLE=1",
        )
        .expect("a bridge party");
    assert_eq!(
        parsed
            .get_by_path(&path("Parties[0].PartyIDSource"))
            .as_ref()
            .and_then(Scalar::as_str),
        Some("D")
    );

    let emitted = parsed.into_bytes(b'|');
    assert!(
        String::from_utf8_lossy(&emitted).contains("|447=D|"),
        "{}",
        String::from_utf8_lossy(&emitted)
    );
    let reread = reader.sole_line(&emitted).expect("the emitted FIX row");
    assert_eq!(
        reread
            .get_by_path(&path("Parties[0].PartyIDSource"))
            .as_ref()
            .and_then(Scalar::as_str),
        Some("D")
    );
}
