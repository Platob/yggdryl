//! `rust/src/fix/crated.rs`: the definitions this crate invents above every
//! published tag.

use super::committed_registry;
use super::fixed_codec;

mod categories {
    use std::sync::Arc;
    use yggdryl::graph::Market;
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
            assert_eq!(message.get_securityids().get("FIGI"), Some("BBG000BLNQ16"));
            assert!(message.get_securityids().get("BLOOMBERG").is_none());
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
        assert!(malformed.get_securityids().get("FIGI").is_none());

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
        assert_eq!(rebuilt.get_securityids().get("FIGI"), Some("BBG000BLNQ16"));
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
    /// own wording; the other eight take the column's.
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
                "recdunix",
                "srcuuids",
                "state",
            ]
        );
    }
}

mod unprojected {
    //! The crate fields that stay content: a bridge's own instrument names,
    //! kept in the row as they arrived and folded into `secaltids`.

    use std::sync::Arc;
    use yggdryl::FixMsg;
    use yggdryl::graph::Market;

    fn parsed(line: &[u8]) -> FixMsg {
        super::fixed_codec(super::committed_registry())
            .parse_fix_line(line)
            .expect("a readable line")
    }

    fn wire(held: &FixMsg) -> String {
        String::from_utf8(held.into_bytes(b'|')).expect("a text wire")
    }

    fn stated(held: &FixMsg, name: &str) -> Option<String> {
        held.get_by_name(name)
            .and_then(|value| value.as_str().map(str::to_owned))
    }

    #[test]
    fn a_bridge_instrument_folds_into_secaltids_once_under_its_own_source() {
        let held = parsed(
            b"8=FIX.4.4|35=8|17=E1|55=HOLN|OMSINSTRUMENTID=dbi;CH0012214059_XSWX_CHF|\
              ULLINK.INSTRUMENTID=dbi;CH0012214059_XSWX_CHF|10=0|",
        );
        let ids = held.get_securityids();
        assert_eq!(
            ids.get("OMSINSTRUMENTID"),
            Some("dbi;CH0012214059_XSWX_CHF")
        );
        assert_eq!(
            ids.get("ULLINKINSTRUMENTID"),
            Some("dbi;CH0012214059_XSWX_CHF")
        );
        // Content, as it arrived: the row states both, the namespaced
        // spelling reached its field rather than the metadata.
        assert_eq!(
            stated(&held, "omsinstrumentid").as_deref(),
            Some("dbi;CH0012214059_XSWX_CHF")
        );
        assert_eq!(
            stated(&held, "ullinkinstrumentid").as_deref(),
            Some("dbi;CH0012214059_XSWX_CHF")
        );
        assert!(held.metadata().is_empty(), "{:?}", held.metadata());
        assert!(held.anomalies().is_empty(), "{:?}", held.anomalies());
        let once = wire(&held);
        assert_eq!(once.matches("456=OMSINSTRUMENTID").count(), 1, "{once}");
        assert_eq!(once.matches("456=ULLINKINSTRUMENTID").count(), 1, "{once}");
        // The wire it writes reads back to the same wire: the fold finds the
        // occurrence it wrote rather than appending a second.
        assert_eq!(wire(&parsed(once.as_bytes())), once);
    }

    #[test]
    fn a_stated_occurrence_under_the_same_source_is_never_overwritten() {
        let held = parsed(
            b"8=FIX.4.4|35=8|17=E1|55=HOLN|454=1|455=OTHER|456=OMSINSTRUMENTID|\
              OMSINSTRUMENTID=dbi;CH0012214059_XSWX_CHF|10=0|",
        );
        assert_eq!(held.get_securityids().get("OMSINSTRUMENTID"), Some("OTHER"));
        let written = wire(&held);
        assert_eq!(
            written.matches("456=OMSINSTRUMENTID").count(),
            1,
            "{written}"
        );
        assert!(written.contains("455=OTHER"), "{written}");
        assert_eq!(
            stated(&held, "omsinstrumentid").as_deref(),
            Some("dbi;CH0012214059_XSWX_CHF")
        );
    }

    #[test]
    fn a_value_past_32_bytes_blocks_the_fold_and_is_an_anomaly() {
        let long = "dbi;CH0012214059_XSWX_CHF_0123456789";
        assert!(long.len() > 32);
        let line = format!("8=FIX.4.4|35=8|17=E1|55=HOLN|OMSINSTRUMENTID={long}|10=0|");
        let held = parsed(line.as_bytes());
        assert!(held.get_securityids().get("OMSINSTRUMENTID").is_none());
        let written = wire(&held);
        assert!(!written.contains("456="), "{written}");
        assert!(
            written.contains(long),
            "the value still re-emits: {written}"
        );
        let anomalies: Vec<&str> = held.anomalies().iter().map(|held| held.field()).collect();
        assert_eq!(anomalies, ["omsinstrumentid"], "{:?}", held.anomalies());
    }

    #[test]
    fn neither_is_a_column_of_the_fixed_row() {
        let registry = super::committed_registry();
        let schema = yggdryl::fix_schema(&registry, "fix").expect("a fixed schema");
        for tag in [
            yggdryl::OMSINSTRUMENTID_TAG_NAME.0,
            yggdryl::ULLINKINSTRUMENTID_TAG_NAME.0,
        ] {
            assert!(!yggdryl::fix_schema_tags().contains(&tag), "{tag}");
            assert!(yggdryl::fix_column_of(&schema, tag).is_none(), "{tag}");
        }
        // A row keeps each among its residual entries, and reads it back.
        let held = parsed(b"8=FIX.4.4|35=8|17=E1|55=HOLN|OMSINSTRUMENTID=dbi;X_Y_Z|10=0|");
        let row = held.into_row(&schema).expect("a row");
        let again = FixMsg::from_row(Arc::clone(&registry), &schema, &row).expect("the message");
        assert_eq!(
            stated(&again, "omsinstrumentid").as_deref(),
            Some("dbi;X_Y_Z")
        );
        assert_eq!(
            again.get_securityids().get("OMSINSTRUMENTID"),
            Some("dbi;X_Y_Z")
        );
    }
}
