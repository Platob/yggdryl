//! `rust/src/fix/crated.rs`: the definitions this crate invents above every
//! published tag.

use super::committed_registry;
use super::fixed_codec;

mod categories {
    use std::sync::Arc;
    use yggdryl::graph::MarketElement;
    use yggdryl::{FIGICode, FixMsg, Scalar};

    #[test]
    fn figi_sources_lift_to_one_typed_crate_column_without_bloomberg_fallback() {
        let registry = super::committed_registry();
        let codec = super::fixed_codec(Arc::clone(&registry));
        let schema = yggdryl::fix_schema(&registry, "fix").expect("a fixed schema");
        for line in [
            b"8=FIX.4.4|35=D|11=P|22=S|48=BBG000BLNQ16|10=0|".as_slice(),
            b"8=FIX.4.4|35=D|11=A|454=1|455=BBG000BLNQ16|456=S|10=0|".as_slice(),
        ] {
            let message = codec.parse_fix_line(line).expect("a FIGI source");
            assert_eq!(
                message.get_figicode().map(|value| value.as_str()),
                Some("BBG000BLNQ16")
            );
            assert!(message.get_bloombergcode().is_none());
            let at = yggdryl::fix_column_of(&schema, yggdryl::FIGICODE_TAG_NAME.0)
                .expect("the FIGI fixed column");
            assert_eq!(
                message.into_row(&schema).unwrap().as_sequence().unwrap()[at].as_str(),
                Some("BBG000BLNQ16")
            );
        }
        let malformed = codec
            .parse_fix_line(b"8=FIX.4.4|35=D|11=X|22=S|48=BBG000BLNQ17|10=0|")
            .expect("a malformed FIGI stays raw");
        assert!(malformed.get_figicode().is_none());

        let mut explicit = codec
            .parse_fix_line(b"8=FIX.4.4|35=D|11=E|10=0|")
            .expect("an empty instrument");
        explicit
            .set(
                yggdryl::FIGICODE_TAG_NAME.0,
                Scalar::FIGICode(FIGICode::new("BBG000BLNQ16").unwrap()),
            )
            .expect("a direct FIGI fact");
        let row = explicit.into_row(&schema).unwrap();
        let rebuilt = FixMsg::from_row(registry, &schema, &row).unwrap();
        assert_eq!(
            rebuilt.get_figicode().map(|value| value.as_str()),
            Some("BBG000BLNQ16")
        );
        assert_eq!(rebuilt.into_row(&schema).unwrap(), row);
    }
}
