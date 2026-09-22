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

mod table {
    use yggdryl::graph::EventColumn;

    /// Every graph event column has a crate definition, and takes that
    /// column's datatype, display and name.
    ///
    /// The table in `rust/src/fix/crated.rs` replaced an exhaustive match on
    /// `EventColumn`, which the compiler used to check. This is that check:
    /// a column added to the graph and not to the table would otherwise have
    /// no crate tag, and a FIX row would answer it under no column at all.
    #[test]
    fn every_event_column_is_one_crate_definition_under_the_columns_own_name() {
        let held = yggdryl::fix_crate_fields().expect("the crate's own fields");
        for column in EventColumn::ALL {
            let found: Vec<_> = held
                .iter()
                .filter(|field| field.name() == column.name())
                .collect();
            assert_eq!(found.len(), 1, "one definition of {}", column.name());
            let field = found[0];
            assert_eq!(
                field.dtype(),
                &column.datatype().expect("the column's datatype"),
                "{} is read at the column's datatype",
                column.name()
            );
            assert_eq!(
                field.display(),
                Some(column.display()),
                "{} displays as the column does",
                column.name()
            );
            let tag = field
                .as_fix()
                .tag()
                .expect("a tag reading")
                .expect("a stated tag");
            assert!(
                yggdryl::is_crate_tag(tag),
                "{} sits in the crate's own block",
                column.name()
            );
        }
    }

    /// The eight columns FIX says more about than the graph does keep their
    /// own wording; the other eleven take the column's.
    ///
    /// Which is which is a judgement, so it is pinned rather than argued: a
    /// description that drifts back into restating the column is caught here
    /// and in the committed dictionary hash.
    #[test]
    fn a_crate_column_restates_the_graphs_wording_only_where_fix_adds_nothing() {
        let held = yggdryl::fix_crate_fields().expect("the crate's own fields");
        let mut own = Vec::new();
        for column in EventColumn::ALL {
            let field = held
                .iter()
                .find(|field| field.name() == column.name())
                .expect("a definition");
            if field.description() != Some(column.description()) {
                own.push(column.name());
            }
        }
        own.sort_unstable();
        assert_eq!(
            own,
            [
                "creaunix",
                "crosscode",
                "currhashcode",
                "execunix",
                "exprtime",
                "identifiers",
                "recdunix",
                "srcuuids",
                "state",
            ]
        );
    }
}
