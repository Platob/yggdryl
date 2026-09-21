//! `rust/src/state.rs`: the coded datatypes FIX's constant vocabulary earns.
//!
//! `DataType` is `#[non_exhaustive]` and the datatype layer carries some sixty
//! wildcard arms, so a new variant compiles clean while behaving wrongly. A
//! green build proves nothing; these are the invariants a wildcard cannot
//! satisfy by accident.

mod coded {

    #[test]
    fn a_state_sorts_from_the_first_state_to_the_terminal_ones() {
        use yggdryl::State;

        // The stored bytes, sorted by nothing but ASCII. This is the whole claim:
        // whatever sorts the column - a Parquet row group's bounds, an external
        // sort, an ORDER BY in something that never heard of this crate - puts
        // every live state before every ended one.
        let mut held: Vec<&str> = yggdryl::StringEnum::STATES.to_vec();
        held.sort_unstable();
        assert_eq!(
            held.as_slice(),
            yggdryl::StringEnum::STATES,
            "the vocabulary is declared in the order it sorts",
        );

        let ordered = [
            "10PENDING",
            "20NEW",
            "40PARTFILL",
            "60PENDCXL",
            "80FILLED",
            "90CANCELED",
            "95REJECTED",
        ];
        let mut shuffled = [
            "95REJECTED",
            "80FILLED",
            "20NEW",
            "60PENDCXL",
            "10PENDING",
            "90CANCELED",
            "40PARTFILL",
        ];
        shuffled.sort_unstable();
        assert_eq!(shuffled, ordered);

        // The rank is the two leading digits, read as the number they spell.
        for (held, rank) in [
            ("00UNKNOWN", 0),
            ("10PENDING", 10),
            ("40PARTFILL", 40),
            ("80FILLED", 80),
            ("90CANCELED", 90),
            ("95REJECTED", 95),
        ] {
            assert_eq!(State::new(held).unwrap().rank(), Some(rank), "{held}");
        }

        // Every ending is told apart from every other without reading a name.
        for held in [
            "10PENDING",
            "20NEW",
            "40PARTFILL",
            "60PENDCXL",
            "70REPLACED",
            "70RESTATED",
        ] {
            assert!(State::new(held).unwrap().is_live(), "{held}");
        }
        assert!(State::new("80FILLED").unwrap().is_done());
        assert!(State::new("90CANCELED").unwrap().is_cancelled());
        assert!(State::new("95REJECTED").unwrap().is_failed());
        for held in ["40PARTFILL", "40TRADE", "80FILLED"] {
            assert!(State::new(held).unwrap().is_execution(), "{held}");
        }
        for held in [
            "40INPROGR",
            "80COMPLETE",
            "40TRDCORR",
            "40TRDCXL",
            "40TRDHOLD",
            "80TRDRELS",
        ] {
            assert!(!State::new(held).unwrap().is_execution(), "{held}");
        }
        for held in ["80FILLED", "90CANCELED", "95REJECTED"] {
            assert!(!State::new(held).unwrap().is_live(), "{held}");
        }

        // The digits between two shipped ranks are placeholders: a state that
        // belongs between them takes one, and the predicates read the band it
        // falls in rather than the exact rank.
        let between = State::new("85ARCHIVED").unwrap();
        assert_eq!(between.rank(), Some(85));
        assert!(between.is_done());
        assert!(!between.is_live());
        assert!(State::new("92HALTED").unwrap().is_cancelled());
        assert!(State::new("97ABORTED").unwrap().is_failed());

        // A value that opens with anything but two digits has no rank, and so is
        // neither live nor ended.
        let unranked = State::new("FILLED").unwrap();
        assert_eq!(unranked.rank(), None);
        assert!(!unranked.is_live());
        assert!(!unranked.is_done());
        assert_eq!(State::new("8FILLED").unwrap().rank(), None);
    }

    #[test]
    fn a_state_answers_a_fix_code_a_fix_name_and_a_scheduler_word_alike() {
        use yggdryl::State;

        // One value, four vocabularies: the wire code an ExecutionReport carries,
        // the specification's name for it, the word a scheduler uses, and the
        // short name a FIX bridge logs.
        for (spelling, expected) in [
            ("0", "20NEW"),
            ("1", "40PARTFILL"),
            ("2", "80FILLED"),
            ("8", "95REJECTED"),
            ("F", "40TRADE"),
            ("New", "20NEW"),
            ("PartiallyFilled", "40PARTFILL"),
            ("DoneForDay", "80DONEDAY"),
            ("done_for_day", "80DONEDAY"),
            ("DONE FOR DAY", "80DONEDAY"),
            ("running", "30RUNNING"),
            ("succeeded", "80SUCCESS"),
            ("timed out", "95TIMEOUT"),
            ("failed", "95FAILED"),
            // The short names a FIX bridge logs fold to the same states.
            ("PartFill", "40PARTFILL"),
            ("PartFilled", "40PARTFILL"),
            ("PendNew", "10PENDNEW"),
            ("PendCancel", "60PENDCXL"),
            ("PendReplace", "60PENDRPL"),
            ("DoneDay", "80DONEDAY"),
            ("Cancel", "90CANCELED"),
            ("Reject", "95REJECTED"),
            // A stored value names itself, so resolving one twice is resolving it
            // once.
            ("80FILLED", "80FILLED"),
        ] {
            let held = State::from_spelling(spelling)
                .unwrap_or_else(|| panic!("{spelling} names no state"));
            assert_eq!(held.as_str(), expected, "{spelling}");
            assert_eq!(
                State::from_spelling(held.as_str()).unwrap().as_str(),
                expected,
                "{spelling} resolves to itself",
            );
        }

        // A wire code never folds: `A` is PendingNew and `a` is not a code at all.
        assert_eq!(State::from_spelling("A").unwrap().as_str(), "10PENDNEW");
        assert_eq!(State::from_spelling("a"), None);
        assert_eq!(State::from_spelling("whatever"), None);
        assert_eq!(State::from_spelling(""), None);
    }
}
