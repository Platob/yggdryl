//! `rust/src/xxhash/arrow.rs` over the enum leaves `yggdryl-market`
//! claims: a column or a row of a market kind digests as the values it
//! holds, through the buffer path as through the value feed, and the corpus
//! names every kind the market claims. Every core datatype's column is
//! `rust/tests/xxhash/arrow.rs`'s.

mod columns {
    use arrow_array::cast::AsArray as _;
    use arrow_array::types::{UInt32Type, UInt64Type};
    use arrow_array::{Array, ArrayRef, FixedSizeBinaryArray, RecordBatch};
    use arrow_schema::{Field as ArrowField, Schema};
    use std::sync::Arc;
    use yggdryl::xxhash::arrow::{column_digests, row_digests};
    use yggdryl::{DataType, Digest, DigestAlgorithm, Field, Scalar, StructType};
    use yggdryl_market::{MarketDataKind, MarketDataType, Side, TimeInForce};

    /// Read one digest column back as the digests it holds.
    fn digests(array: &ArrayRef, algorithm: DigestAlgorithm) -> Vec<Digest> {
        match algorithm {
            DigestAlgorithm::Xxh32 => array
                .as_primitive::<UInt32Type>()
                .values()
                .iter()
                .map(|value| Digest::new(algorithm, u128::from(*value)))
                .collect(),
            DigestAlgorithm::Xxh64 | DigestAlgorithm::Xxh3 => array
                .as_primitive::<UInt64Type>()
                .values()
                .iter()
                .map(|value| Digest::new(algorithm, u128::from(*value)))
                .collect(),
            DigestAlgorithm::Xxh128 => {
                let array = array
                    .as_any()
                    .downcast_ref::<FixedSizeBinaryArray>()
                    .expect("the 128-bit column is fixed-size binary");
                (0..array.len())
                    .map(|index| {
                        Digest::from_bytes(algorithm, array.value(index)).expect("the exact width")
                    })
                    .collect()
            }
            // `DigestAlgorithm` is non-exhaustive outside the crate; a new one
            // reaching here has no column reading stated yet.
            other => panic!("{other} has no digest column reading"),
        }
    }

    /// One column per kind the market claims, each with the values that
    /// exercise it.
    fn columns() -> Vec<(Field, Scalar)> {
        vec![
            (
                Side::field("side"),
                Scalar::from_sequence([Scalar::from("BUY"), Scalar::from("SELL"), Scalar::Null]),
            ),
            (
                MarketDataKind::field("marketdatakind"),
                Scalar::from_sequence([Scalar::from("ORDR"), Scalar::from("TRAD"), Scalar::Null]),
            ),
            (
                MarketDataType::field("marketdatatype"),
                Scalar::from_sequence([
                    Scalar::from("ORDLIMIT"),
                    Scalar::from("TRDBLOCK"),
                    Scalar::Null,
                ]),
            ),
            (
                TimeInForce::field("timeinforce"),
                Scalar::from_sequence([Scalar::from("0"), Scalar::from("6"), Scalar::Null]),
            ),
            (
                Side::field("pluginside"),
                Scalar::from_sequence([
                    Scalar::from("BUYS"),
                    Scalar::from("sell-side"),
                    Scalar::Null,
                ]),
            ),
        ]
    }

    #[test]
    fn a_column_digest_equals_the_value_feed_on_every_market_kind() {
        crate::install::installed();
        for (field, values) in columns() {
            let values = values.as_sequence().expect("a sequence of values").to_vec();
            let array = yggdryl::Serie::from_scalars(field.clone(), values)
                .and_then(|serie| serie.require_arrow_array())
                .unwrap_or_else(|error| panic!("{}: {error}", field.name()));
            // Read the values back through the shared boundary rather than reusing
            // the input, so a column that canonicalizes on the way in is compared
            // against what it actually stores.
            let stored = yggdryl::Serie::from_arrow_array(
                Some(&field),
                Arc::clone(&array),
                yggdryl::ArrowCastOptions::default(),
            )
            .map(Scalar::from)
            .unwrap_or_else(|error| panic!("{}: {error}", field.name()));
            let stored = stored.sequence_rows().expect("a sequence of values");

            for algorithm in DigestAlgorithm::ALL {
                let column = column_digests(Arc::clone(&array), &field, algorithm)
                    .unwrap_or_else(|error| panic!("{}: {error}", field.name()));
                assert_eq!(column.len(), stored.len(), "{}", field.name());
                assert_eq!(
                    digests(&column, algorithm),
                    stored
                        .iter()
                        .map(|value| value.digest(algorithm))
                        .collect::<Vec<_>>(),
                    "{} under {algorithm}",
                    field.name()
                );
            }
        }
    }

    #[test]
    fn a_row_digest_equals_the_row_value_feed_on_the_market_kinds() {
        crate::install::installed();
        // One batch holding every kind at once, so the row framing is exercised
        // across the buffer path and the fallback in the same row.
        let width = columns()
            .iter()
            .map(|(_, values)| values.len())
            .max()
            .expect("the corpus is not empty");
        let mut fields = Vec::new();
        let mut arrays: Vec<ArrayRef> = Vec::new();
        for (field, values) in columns() {
            // Pad every column to the same height, which also puts a null in
            // every family that can hold one. A union stores its validity in the
            // child rather than the parent, so it repeats a real member instead.
            let mut padded = values.as_sequence().expect("a sequence").to_vec();
            let filler = if matches!(field.dtype(), DataType::Union(..)) {
                padded
                    .first()
                    .cloned()
                    .expect("the union column is not empty")
            } else {
                Scalar::Null
            };
            padded.resize(width, filler);
            let field = field.clone().with_nullable(true);
            let array = yggdryl::Serie::from_scalars(field.clone(), padded)
                .and_then(|serie| serie.require_arrow_array())
                .unwrap_or_else(|error| panic!("{}: {error}", field.name()));
            fields.push(field);
            arrays.push(array);
        }
        let root = DataType::from(StructType::from_fields(fields).unwrap()).required_field("row");
        let arrow_fields: Vec<ArrowField> = root
            .dtype()
            .as_fields()
            .expect("a struct root")
            .iter()
            .map(|field| field.clone().into_arrow_field())
            .collect::<yggdryl::Result<Vec<_>>>()
            .unwrap();
        let batch = RecordBatch::try_new(Arc::new(Schema::new(arrow_fields)), arrays).unwrap();

        let rows =
            yggdryl::Serie::from_arrow_batch(None, &batch, yggdryl::ArrowCastOptions::default())
                .map(Scalar::from)
                .unwrap();
        let rows = rows.sequence_rows().expect("a sequence of rows");
        assert_eq!(rows.len(), width);

        for algorithm in DigestAlgorithm::ALL {
            let column = row_digests(&batch, algorithm).unwrap();
            assert_eq!(column.len(), width);
            assert_eq!(
                digests(&column, algorithm),
                rows.iter()
                    .map(|row| row.digest(algorithm))
                    .collect::<Vec<_>>(),
                "{algorithm}"
            );
        }
    }

    #[test]
    fn the_corpus_names_every_kind_the_market_claims() {
        crate::install::installed();
        let covered: std::collections::HashSet<_> =
            columns().iter().map(|(field, _)| field.id()).collect();
        for kind in yggdryl::market::kinds() {
            assert!(
                covered.contains(&kind.id),
                "{} is missing from the digest corpus",
                kind.id
            );
        }
    }
}
