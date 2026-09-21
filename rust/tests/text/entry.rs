//! `rust/src/text/entry.rs`: the key/value tree one text line carries.

mod text {

    /// One hash of any hashable value, for "equal values hash alike" and for a
    /// temporary name that does not collide. The crate's own stable hash is
    /// private, and neither use needs it to be stable across runs.
    pub(super) fn hash_of<T: std::hash::Hash>(value: &T) -> u64 {
        use std::hash::Hasher as _;

        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        value.hash(&mut hasher);
        hasher.finish()
    }

    // --- The decoded row value and its parts ---
    mod values {
        use super::hash_of;
        use std::sync::Arc;
        use yggdryl::text::{TextBytes, TextEntries, TextEntry, TextLine};
        use yggdryl::{FieldPath, FieldSegment};

        fn page(bytes: &[u8]) -> Arc<Vec<u8>> {
            Arc::new(bytes.to_vec())
        }

        fn path(text: &str) -> FieldPath {
            FieldPath::from_str(text).expect("path parses")
        }

        fn entry(key: &str, value: &str) -> TextEntry {
            TextEntry::new(
                TextBytes::from_bytes(key).expect("a key"),
                TextBytes::from_bytes(value).expect("a value"),
            )
        }

        /// Every pair a body declares, rendered as the line wrote it.
        fn read(body: &[u8]) -> Vec<String> {
            let body = TextBytes::from_bytes(body).expect("a body");
            yggdryl::text::TextEntries::from_bytes(&body)
                .as_ref()
                .map(TextEntries::as_slice)
                .unwrap_or_default()
                .iter()
                .map(ToString::to_string)
                .collect()
        }

        fn tree(body: &[u8]) -> TextEntries {
            let body = TextBytes::from_bytes(body).expect("a body");
            yggdryl::text::TextEntries::from_bytes(&body).expect("the line states pairs")
        }

        #[test]
        fn an_entry_tree_is_found_by_path_at_every_depth() {
            let nested = TextEntries::from_iter([entry("PartyID", "ACME")]);
            let entries = TextEntries::from_iter([
                entry("35", "D"),
                entry("55", "AAPL"),
                TextEntry::new(
                    TextBytes::from_bytes("213").expect("a key"),
                    TextBytes::new(),
                )
                .with_entries(nested),
            ]);
            let line = TextLine::from_bytes(
                0,
                TextBytes::from_bytes("body").expect("a body"),
                std::sync::Arc::new(yggdryl::text::TextOptions::new()),
            )
            .unwrap()
            .with_entries(entries);

            assert_eq!(
                line.get_entry_by_path(&path("55"))
                    .and_then(|held| held.value_bytes().as_str()),
                Some("AAPL")
            );
            assert_eq!(
                line.get_entry_by_path(&path("\"213\".PartyID"))
                    .and_then(|held| held.value_bytes().as_str()),
                Some("ACME")
            );
            assert_eq!(
                line.get_entry_by_path(&path("[1]"))
                    .and_then(|held| held.key_bytes().as_str()),
                Some("55")
            );
            assert_eq!(
                line.get_entry_by_path(&path("[-1]"))
                    .and_then(|held| held.key_bytes().as_str()),
                Some("213")
            );
        }

        #[test]
        fn a_miss_is_null_and_never_an_error_or_a_panic() {
            let line = TextLine::from_bytes(
                0,
                TextBytes::from_bytes("body").expect("a body"),
                std::sync::Arc::new(yggdryl::text::TextOptions::new()),
            )
            .unwrap()
            .with_entries(TextEntries::from_iter([entry("a", "1")]));
            for text in ["b", "a.b", "a.b.c", "[9]", "[-9]"] {
                assert!(
                    line.get_entry_by_path(&path(text)).is_none(),
                    "{text} must miss"
                );
            }
            // A line carrying no tree at all misses the same way.
            let bare = TextLine::from_bytes(
                0,
                TextBytes::from_bytes("body").expect("a body"),
                std::sync::Arc::new(yggdryl::text::TextOptions::new()),
            )
            .unwrap();
            assert!(bare.get_entry_by_path(&path("a")).is_none());
            // The raising form says why, and names the path.
            let error = bare.entry_by_path(&path("a")).expect_err("raises");
            assert!(error.to_string().contains('a'), "{error}");
        }

        #[test]
        fn a_key_with_no_value_is_found_and_is_not_a_miss() {
            let line = TextLine::from_bytes(
                0,
                TextBytes::from_bytes("body").expect("a body"),
                std::sync::Arc::new(yggdryl::text::TextOptions::new()),
            )
            .unwrap()
            .with_entries(TextEntries::from_iter([entry("a", "")]));
            let found = line
                .get_entry_by_path(&path("a"))
                .expect("the key is there");
            assert!(found.value().is_empty());
        }

        #[test]
        fn the_setter_creates_what_is_not_there() {
            let mut line = TextLine::from_bytes(
                0,
                TextBytes::from_bytes("body").expect("a body"),
                std::sync::Arc::new(yggdryl::text::TextOptions::new()),
            )
            .unwrap();
            line.set_entry_by_path(
                &path("order.price"),
                TextBytes::from_bytes("12").expect("a value"),
            )
            .expect("creates both levels");
            assert_eq!(
                line.get_entry_by_path(&path("order.price"))
                    .and_then(|held| held.value_bytes().as_str()),
                Some("12")
            );
            // Setting again replaces rather than appending a second entry.
            line.set_entry_by_path(
                &path("order.price"),
                TextBytes::from_bytes("13").expect("a value"),
            )
            .expect("replaces");
            assert_eq!(
                line.entries().map(TextEntries::len),
                Some(1),
                "one root entry"
            );
            assert_eq!(
                line.get_entry_by_path(&path("order.price"))
                    .and_then(|held| held.value_bytes().as_str()),
                Some("13")
            );
        }

        #[test]
        fn a_position_naming_no_entry_refuses_and_changes_nothing() {
            let mut line = TextLine::from_bytes(
                0,
                TextBytes::from_bytes("body").expect("a body"),
                std::sync::Arc::new(yggdryl::text::TextOptions::new()),
            )
            .unwrap()
            .with_entries(TextEntries::from_iter([entry("a", "1")]));
            let before = line.clone();
            let error = line
                .set_entry_by_path(&path("[4]"), TextBytes::from_bytes("x").expect("a value"))
                .expect_err("a position names an entry that exists");
            assert!(error.to_string().contains("position"), "{error}");
            assert_eq!(line, before, "a refusal leaves the line unchanged");
        }

        #[test]
        fn the_root_path_is_not_a_place_to_set() {
            let mut line = TextLine::from_bytes(
                0,
                TextBytes::from_bytes("body").expect("a body"),
                std::sync::Arc::new(yggdryl::text::TextOptions::new()),
            )
            .unwrap();
            assert!(
                line.set_entry_by_path(&FieldPath::root(), TextBytes::new())
                    .is_err()
            );
        }

        #[test]
        fn removing_takes_one_entry_at_the_path() {
            let mut line = TextLine::from_bytes(
                0,
                TextBytes::from_bytes("body").expect("a body"),
                std::sync::Arc::new(yggdryl::text::TextOptions::new()),
            )
            .unwrap()
            .with_entries(TextEntries::from_iter([entry("a", "1"), entry("b", "2")]));
            let removed = line.remove_entry_by_path(&path("a")).expect("removes");
            assert_eq!(removed.key_bytes().as_str(), Some("a"));
            assert_eq!(line.entries().map(TextEntries::len), Some(1));
            assert!(line.remove_entry_by_path(&path("a")).is_none());
        }

        #[test]
        fn a_repeated_key_is_two_entries_reachable_by_position() {
            let line = TextLine::from_bytes(
                0,
                TextBytes::from_bytes("body").expect("a body"),
                std::sync::Arc::new(yggdryl::text::TextOptions::new()),
            )
            .unwrap()
            .with_entries(TextEntries::from_iter([
                entry("tag", "first"),
                entry("tag", "second"),
            ]));
            assert_eq!(
                line.get_entry_by_path(&path("tag"))
                    .and_then(|held| held.value_bytes().as_str()),
                Some("first"),
                "a name reaches the first"
            );
            assert_eq!(
                line.get_entry_by_path(&path("[1]"))
                    .and_then(|held| held.value_bytes().as_str()),
                Some("second")
            );
        }

        #[test]
        fn a_line_carries_what_no_other_field_can_recover() {
            let mut line = TextLine::from_bytes(
                7,
                TextBytes::from_bytes("hello").expect("a body"),
                std::sync::Arc::new(yggdryl::text::TextOptions::new()),
            )
            .unwrap();
            assert_eq!(line.index(), 7);
            line.set_handle_mtime(Some(1_700_000_000_000_000_000));
            line.set_dropped_byte_size(Some(12));
            assert_eq!(line.mtime().unwrap(), Some(1_700_000_000_000_000_000));
            assert_eq!(line.dropped_byte_size(), Some(12));
            assert_eq!(line.body(), "hello");
        }

        #[test]
        fn a_path_segment_naming_nothing_addressable_misses_rather_than_panics() {
            let line = TextLine::from_bytes(
                0,
                TextBytes::from_bytes("body").expect("a body"),
                std::sync::Arc::new(yggdryl::text::TextOptions::new()),
            )
            .unwrap()
            .with_entries(TextEntries::from_iter([entry("a", "1")]));
            let by_index = FieldPath::new([FieldSegment::index(0)]);
            assert!(line.get_entry_by_path(&by_index).is_some());
            let deep = FieldPath::new([FieldSegment::index(0), FieldSegment::field("x")]);
            assert!(line.get_entry_by_path(&deep).is_none());
        }

        #[test]
        fn a_tree_read_from_a_frame_keeps_the_whole_value_the_frame_bounded() {
            assert_eq!(
                read(b"8=FIX.4.4|35=D|18=G L|58=quoting #A=1 and #B=2|10=0|"),
                [
                    "8=FIX.4.4",
                    "35=D",
                    "18=G L",
                    "58=quoting #A=1 and #B=2",
                    "10=0",
                ]
            );
            let entries = tree(b"8=FIX.4.4|35=D|NoAllocs[0].79=ACCT|Symbol[0]=AAPL|10=0|");
            assert_eq!(
                entries.as_slice()[2].key_bytes().as_str(),
                Some("NoAllocs[0].79"),
                "an indexed key is a key, and the loose walk found neither of these"
            );
            assert_eq!(
                entries.as_slice()[3].key_bytes().as_str(),
                Some("Symbol[0]")
            );
        }

        #[test]
        fn a_text_field_quoting_pairs_is_one_field_carrying_a_tree_of_them() {
            // The frame decides where the Text field ends; what that field's own
            // text says is read under it, which is what a pair-shaped value has
            // always meant here. The two readings do not compete: `58` is one
            // field of the frame, and `A` is a member of what `58` says.
            let entries = tree(b"8=FIX.4.4|35=D|58=quoting #A=1 and #B=2|10=0|");
            let quoting = &entries.as_slice()[2];
            assert_eq!(
                quoting.value_bytes().as_str(),
                Some("quoting #A=1 and #B=2")
            );
            let quoted = quoting
                .entries()
                .expect("the value states pairs")
                .as_slice();
            assert_eq!(quoted.len(), 2);
            assert_eq!(quoted[0].key_bytes().as_str(), Some("A"));
            assert!(
                quoted[0].marked() && quoted[1].marked(),
                "the quoted keys carry the mark the text wrote in front of them"
            );
            assert_eq!(
                entries
                    .get_entry_by_path(&path("58"))
                    .and_then(|found| found.value_bytes().as_str()),
                Some("quoting #A=1 and #B=2"),
                "and the frame's own field is what the frame's own key reaches"
            );
        }

        #[test]
        fn a_value_a_frame_bounded_is_still_a_range_of_the_page_it_came_from() {
            let page = page(b"8=FIX.4.4|58=a value with spaces|10=0|");
            let body = TextBytes::from_whole_page(Arc::clone(&page)).expect("the whole page");
            let entries = yggdryl::text::TextEntries::from_bytes(&body).expect("pairs");
            let held = &entries.as_slice()[1];
            assert_eq!(held.value(), "a value with spaces");
            assert!(
                Arc::ptr_eq(held.value_bytes().page().expect("a page"), &page),
                "a wider value is a wider range, never a copy"
            );
        }

        #[test]
        fn an_entry_records_the_mark_the_line_wrote_and_keeps_its_key_stripped() {
            let entries = tree(b"MSGTYPE=D|ORDERID=123|#ORDERID=123|#SIDE=1");
            let held = entries.as_slice();
            assert!(!held[1].marked() && held[2].marked());
            assert_eq!(
                held[1].key_bytes().as_str(),
                held[2].key_bytes().as_str(),
                "the mark is off the key, so a path still lifts the name the \
             bridge gave the field"
            );
            assert_eq!(
                entries
                    .get_entry_by_path(&path("ORDERID"))
                    .and_then(|found| found.value_bytes().as_str()),
                Some("123"),
                "and the name reaches the pair that arrived first"
            );
            assert!(
                held[3].marked() && !held[3].key_bytes().as_bytes().starts_with(b"#"),
                "a marked key with no bare twin is marked all the same"
            );
        }

        #[test]
        fn two_entries_differing_only_in_the_mark_are_two_values() {
            let equal = tree(b"MSGTYPE=D|ORDERID=123|#ORDERID=123");
            let (bare, marked) = (&equal.as_slice()[1], &equal.as_slice()[2]);
            assert_eq!(bare.key(), marked.key());
            assert_eq!(bare.value(), marked.value());
            assert_ne!(bare, marked, "the line wrote two different things");
            assert_ne!(
                hash_of(bare),
                hash_of(marked),
                "unequal values must not be forced to hash alike"
            );
            assert!(
                bare < marked,
                "a bare pair sorts in front of its restatement"
            );
            assert_eq!(marked.to_string(), "#ORDERID=123");
            assert_eq!(bare.to_string(), "ORDERID=123");

            let differing = tree(b"MSGTYPE=D|ORDERID=123|#ORDERID=345");
            assert!(differing.as_slice()[2].marked());
            assert_eq!(differing.as_slice()[2].value_bytes().as_str(), Some("345"));
        }

        #[test]
        fn a_marked_stem_and_its_occurrence_arrive_marked_and_nested() {
            let entries = tree(b"MSGTYPE=D|#NOPARTYIDS=3|#NOPARTYIDS[0]=PARTYID=ONE");
            let held = entries.as_slice();
            assert!(held[1].marked() && held[2].marked());
            assert_eq!(held[1].key_bytes().as_str(), Some("NOPARTYIDS"));
            assert_eq!(held[2].key_bytes().as_str(), Some("NOPARTYIDS[0]"));
            assert_eq!(
                held[2]
                    .entries()
                    .and_then(|nested| nested.as_slice().first())
                    .and_then(|member| member.value_bytes().as_str()),
                Some("ONE"),
                "an occurrence's members are read in their own scope"
            );
        }

        #[test]
        fn a_stated_absence_arrives_under_every_spelling_of_it() {
            assert_eq!(
                read(b"MSGTYPE=D|SYMBOL=|SIDE=null|PRICE=<null>|ACCOUNT=A"),
                [
                    "MSGTYPE=D",
                    "SYMBOL=",
                    "SIDE=null",
                    "PRICE=<null>",
                    "ACCOUNT=A",
                ],
                "the tree carries what the line wrote; which spelling means absent \
             is a dialect's reading of it"
            );
            assert!(
                tree(b"MSGTYPE=D|SYMBOL=|SIDE=1").as_slice()[1]
                    .value()
                    .is_empty()
            );
        }

        #[test]
        fn an_entry_nothing_wrote_a_mark_for_is_unmarked() {
            let mut entries = TextEntries::new();
            entries
                .set_entry_by_path(
                    &path("created"),
                    TextBytes::from_bytes("x").expect("a value"),
                )
                .expect("the path names a child");
            assert!(
                !entries.as_slice()[0].marked(),
                "a caller creating an entry states no mark, and none is invented"
            );
        }
    }
}
