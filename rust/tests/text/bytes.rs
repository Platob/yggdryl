//! `rust/src/text/bytes.rs`: a counted range of bytes over one retained page.

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
        use yggdryl::FieldPath;
        use yggdryl::text::{TextBytes, TextEntries, TextEntry};

        fn page(bytes: &[u8]) -> Arc<Vec<u8>> {
            Arc::new(bytes.to_vec())
        }

        fn path(text: &str) -> FieldPath {
            FieldPath::from_str(text).expect("path parses")
        }

        #[test]
        fn an_entry_answers_text_borrowed_from_its_page_and_its_range_beside_it() {
            use std::borrow::Cow;

            let body = TextBytes::from_bytes("8=FIX.4.4|58=caf\u{e9}|10=0|".as_bytes()).unwrap();
            let entries = TextEntries::from_bytes(&body).unwrap();
            let text = &entries.as_slice()[1];
            assert!(matches!(text.key(), Cow::Borrowed("58")));
            assert!(matches!(text.value(), Cow::Borrowed("caf\u{e9}")));
            assert_eq!(text.value_bytes().as_bytes(), "caf\u{e9}".as_bytes());
            assert!(
                std::sync::Arc::ptr_eq(text.value_bytes().page().unwrap(), body.page().unwrap()),
                "the range is the line's own page"
            );
            assert_eq!(text.to_string(), "58=caf\u{e9}");

            // A range a caller built from bytes that are not text is the one
            // case the answer is owned: the lossy decode, and the range intact.
            let raw = TextEntry::new(
                TextBytes::from_bytes(b"96").unwrap(),
                TextBytes::from_bytes(b"\xff\xfe A").unwrap(),
            );
            assert!(matches!(raw.value(), Cow::Owned(_)));
            assert_eq!(raw.value(), "\u{fffd}\u{fffd} A");
            assert_eq!(raw.value_bytes().as_bytes(), b"\xff\xfe A");
            assert!(
                TextEntries::from_iter([raw.clone()])
                    .get_entry_by_path(&path("96"))
                    .is_some(),
                "and its key, which is text, is reachable by name"
            );
        }

        #[test]
        fn a_range_borrows_its_page_and_copies_nothing() {
            let page = page(b"alpha beta gamma");
            let middle = TextBytes::from_page(&page, 6, 10).expect("inside the page");
            assert_eq!(middle.as_bytes(), b"beta");
            assert_eq!(middle.len(), 4);
            assert_eq!(middle.start(), 6);
            assert_eq!(middle.end(), 10);
            // The range points into the very page it was taken from.
            assert!(Arc::ptr_eq(middle.page().expect("a page"), &page));
        }

        #[test]
        fn an_empty_range_retains_no_page() {
            let page = page(b"alpha");
            let empty = TextBytes::from_page(&page, 2, 2).expect("an empty range");
            assert!(empty.is_empty());
            assert_eq!(empty.as_bytes(), b"");
            assert!(empty.page().is_none(), "an empty range pins nothing");
            assert_eq!(Arc::strong_count(&page), 1);
        }

        #[test]
        fn a_range_outside_its_page_is_refused() {
            let page = page(b"alpha");
            assert!(TextBytes::from_page(&page, 0, 6).is_err());
            assert!(TextBytes::from_page(&page, 4, 2).is_err());
            assert!(TextBytes::from_page(&page, 0, 5).is_ok());
        }

        #[test]
        fn identity_is_the_bytes_and_never_the_page() {
            let left = TextBytes::from_page(&page(b"xxbetayy"), 2, 6).expect("inside");
            let right = TextBytes::from_page(&page(b"beta"), 0, 4).expect("inside");
            assert_eq!(left, right, "same bytes, different pages");
            assert_eq!(
                hash_of(&left),
                hash_of(&right),
                "equal values must hash alike"
            );
            assert!(left <= right && right <= left);
        }

        #[test]
        fn slicing_stays_inside_the_same_page() {
            let page = page(b"alpha beta gamma");
            let tail = TextBytes::from_page(&page, 6, 16).expect("inside");
            let inner = tail.slice(0, 4).expect("inside the range");
            assert_eq!(inner.as_bytes(), b"beta");
            assert!(Arc::ptr_eq(inner.page().expect("a page"), &page));
            assert!(tail.slice(0, 99).is_err());
        }
    }
}
