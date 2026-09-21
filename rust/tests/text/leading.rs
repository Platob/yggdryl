//! `rust/src/text/leading.rs`: the physical lines before the first framed
//! record header.

mod text {
    use arrow_array::{Array as _, Int64Array, StringArray};
    use yggdryl::holder::Buffer;

    use yggdryl::text::{LeadingFragment, TextOptions};

    use yggdryl::IOMedia as _;

    fn named(name: &str, bytes: &[u8]) -> Buffer {
        Buffer::from_bytes(bytes.to_vec()).with_media_type(
            yggdryl::Url::from_str(&format!("file:///{name}"))
                .unwrap()
                .media_type(),
        )
    }

    fn options(rowheader: &str) -> TextOptions {
        TextOptions::new().try_with_rowheader(rowheader).unwrap()
    }

    fn framed(rowheader: &str) -> TextOptions {
        options(rowheader).with_framing(true)
    }

    fn collect(
        source: &impl yggdryl::IOBase,
        options: TextOptions,
    ) -> Vec<arrow_array::RecordBatch> {
        source
            .read_arrow_reader(&options.into())
            .unwrap()
            .collect::<std::result::Result<Vec<_>, _>>()
            .unwrap()
    }

    fn bodies(batches: &[arrow_array::RecordBatch]) -> Vec<Vec<u8>> {
        batches
            .iter()
            .flat_map(|batch| {
                let index = batch.schema().index_of("body").unwrap();
                batch
                    .column(index)
                    .as_any()
                    .downcast_ref::<StringArray>()
                    .unwrap()
                    .iter()
                    .map(|value| value.unwrap().as_bytes().to_vec())
                    .collect::<Vec<_>>()
            })
            .collect()
    }

    fn rownums(batches: &[arrow_array::RecordBatch]) -> Vec<i64> {
        batches
            .iter()
            .flat_map(|batch| {
                let index = batch.schema().index_of("rownum").unwrap();
                batch
                    .column(index)
                    .as_any()
                    .downcast_ref::<Int64Array>()
                    .unwrap()
                    .values()
                    .to_vec()
            })
            .collect()
    }

    #[test]
    fn leading_fragments_are_kept_dropped_or_rejected_and_eof_finishes_a_record() {
        let source = named("leading.log", b"before\nstill before\n[A] final");

        let mut keep = framed(r"^\[(?<kind>[A-Z])\] ");
        keep.start_rownum = Some(1);
        let kept = collect(&source, keep);
        assert_eq!(
            bodies(&kept),
            [b"before\nstill before".to_vec(), b"[A] final".to_vec()]
        );
        assert_eq!(rownums(&kept), [1, 3]);
        let kind = kept[0]
            .column(kept[0].schema().index_of("kind").unwrap())
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap();
        assert!(kind.is_null(0));

        let dropped = collect(
            &source,
            framed(r"^\[(?<kind>[A-Z])\] ").with_leading_fragment(LeadingFragment::Drop),
        );
        assert_eq!(bodies(&dropped), [b"[A] final".to_vec()]);

        let rejected =
            framed(r"^\[(?<kind>[A-Z])\] ").with_leading_fragment(LeadingFragment::Error);
        let mut reader = source.read_arrow_reader(&rejected.into()).unwrap();
        let error = reader.next().unwrap().unwrap_err().to_string();
        assert!(error.contains("leading physical line"));
        assert!(reader.next().is_none());
    }
}
