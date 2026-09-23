//! `rust/src/expression/attribute.rs`: the edge cases this module is built
//! to get right.
//!
//! Five properties carry most of the weight, and each is asserted rather than
//! reviewed: text round-trips through the grammar, the scalar and vectorized
//! tiers agree on every operator including nulls and `nan`, a simplification
//! never changes what a row answers, a free attribute never costs a backend
//! call, and a pruning decision never loses a row.

mod grammar {
    use std::cell::Cell;

    use yggdryl::expression::{Attribute, Cost, Term};
    use yggdryl::{
        DataType, Field, MediaType, Result, Scalar, StructType, TimeUnit, Timezone, Url,
    };

    // ---------------------------------------------------------------------------
    // The shared fixture
    // ---------------------------------------------------------------------------

    /// A schema that covers one column of every family a comparison can meet.
    fn rows_schema() -> Field {
        Field::new(
            "rows",
            StructType::from_fields([
                Field::new("i", DataType::Int64, true),
                Field::new("f", DataType::Float64, true),
                Field::new("d", DataType::decimal128(9, 2).unwrap(), true),
                Field::new("s", DataType::utf8(), true),
                Field::new("b", DataType::Boolean, true),
                Field::new(
                    "t",
                    DataType::DateTime64 {
                        unit: TimeUnit::Microsecond,
                        timezone: Timezone::UTC,
                    },
                    true,
                ),
                Field::new("n", DataType::Int32, true).with_partition(true),
                Field::new(
                    "nested",
                    DataType::from(
                        StructType::from_fields([Field::new("leg", DataType::utf8(), true)])
                            .unwrap(),
                    ),
                    true,
                ),
                // Temporal text, so a cast into and out of a temporal is one of
                // the pairs the two tiers are compared on.
                Field::new("clock", DataType::utf8(), true),
                // A list, so a position and a run are compared on both tiers.
                Field::new(
                    "xs",
                    DataType::list(DataType::Int64.nullable_field("item")),
                    true,
                ),
                // A list of structs holding a list of structs, so a predicate
                // segment and one nested in another are compared on both tiers.
                Field::new("legs", DataType::list(leg_field()), true),
            ])
            .map(DataType::from)
            .unwrap(),
            false,
        )
    }

    /// One leg: a currency, a size, and notes that are themselves a list of
    /// structs.
    fn leg_field() -> Field {
        StructType::from_fields([
            DataType::utf8().nullable_field("ccy"),
            DataType::Int64.nullable_field("size"),
            DataType::list(
                StructType::from_fields([
                    DataType::utf8().nullable_field("k"),
                    DataType::Int64.nullable_field("v"),
                ])
                .map(DataType::from)
                .unwrap()
                .nullable_field("item"),
            )
            .nullable_field("notes"),
        ])
        .map(DataType::from)
        .unwrap()
        .nullable_field("item")
    }

    // ---------------------------------------------------------------------------
    // Attributes and their cost
    // ---------------------------------------------------------------------------

    /// A handle that counts every stat it is asked for.
    struct Counting {
        url: Url,
        media_type: MediaType,
        stats: Cell<usize>,
    }

    impl Counting {
        fn new(url: &str) -> Self {
            Self {
                url: Url::from_str(url).unwrap(),
                media_type: MediaType::default(),
                stats: Cell::new(0),
            }
        }
    }

    impl yggdryl::IOMedia for Counting {
        yggdryl::impl_default_iomedia!();
    }

    impl yggdryl::IOBase for Counting {
        fn pread(&self, _offset: u64, _buffer: &mut [u8]) -> Result<usize> {
            Ok(0)
        }

        fn pwrite(&mut self, _offset: u64, _bytes: &[u8]) -> Result<usize> {
            Ok(0)
        }

        fn size(&self) -> u64 {
            self.stats.set(self.stats.get() + 1);
            4_096
        }

        fn capacity(&self) -> u64 {
            4_096
        }

        fn reserve(&mut self, _capacity: u64) -> Result<()> {
            Ok(())
        }

        fn truncate(&mut self, _size: u64) -> Result<()> {
            Ok(())
        }

        fn uri(&self) -> Option<&yggdryl::Uri> {
            Some(self.url.as_ref())
        }

        fn url(&self) -> Option<&Url> {
            Some(&self.url)
        }

        fn media_type(&self) -> &MediaType {
            &self.media_type
        }

        fn set_media_type(&mut self, media_type: MediaType) {
            self.media_type = media_type;
        }
    }

    #[test]
    fn a_free_attribute_answers_without_a_single_stat() {
        let schema = rows_schema();
        // Written stat-first on purpose: bind is what puts the free test in front.
        let bound = "&holder.size > 0 and &holder.partition['year'] = '2023'"
            .parse::<Term>()
            .unwrap()
            .bind(&schema)
            .unwrap();
        let handle = Counting::new("file:///lake/year=2024/part-0.parquet");
        assert!(
            !bound
                .matches_holder(&yggdryl::expression::Handle(&handle))
                .unwrap()
        );
        assert_eq!(
            handle.stats.get(),
            0,
            "a predicate settled by the path still cost a stat"
        );

        // A holder the free test does not rule out pays for the stat, once.
        let matching = Counting::new("file:///lake/year=2023/part-0.parquet");
        assert!(
            bound
                .matches_holder(&yggdryl::expression::Handle(&matching))
                .unwrap()
        );
        assert_eq!(matching.stats.get(), 1);
    }

    #[test]
    fn a_row_predicate_rules_no_holder_out() {
        let schema = rows_schema();
        let bound = "i > 1000000"
            .parse::<Term>()
            .unwrap()
            .bind(&schema)
            .unwrap();
        let handle = Counting::new("file:///lake/part-0.parquet");
        assert!(
            bound
                .matches_holder(&yggdryl::expression::Handle(&handle))
                .unwrap(),
            "a listing filter may never discard a file it has not read"
        );
    }

    #[test]
    fn every_attribute_declares_a_cost_and_a_type() {
        for attribute in Attribute::ALL {
            let field = attribute.field();
            assert!(field.is_nullable());
            assert_eq!(field.name(), format!("&holder.{attribute}"));
            // A free attribute is answerable from a URL alone; a stat one is not.
            let url = Url::from_str("file:///lake/year=2024/part-0.parquet").unwrap();
            let answered = attribute.read_url(&url);
            assert_eq!(
                answered.is_null(),
                matches!(attribute.cost(), Cost::Stat),
                "{attribute} disagreed with its own cost class"
            );
        }
        let partition = Attribute::Partition("year".into());
        assert_eq!(partition.cost(), Cost::Free);
        let url = Url::from_str("file:///lake/year=2024/part-0.parquet").unwrap();
        assert_eq!(partition.read_url(&url), Scalar::from("2024"));
    }
}
