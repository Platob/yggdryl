//! `rust/src/text/sep.rs`: the record terminator - optional, flexible when
//! unset, exact when pinned.

mod text {
    use arrow_array::{Array as _, Int64Array, StringArray};
    use yggdryl::holder::Buffer;
    use yggdryl::media::IORecordOptions as _;
    use yggdryl::text::TextOptions;

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
    fn framing_normalizes_every_physical_terminator_and_keeps_start_rownums() {
        let source = named(
            "mixed.log",
            b"[A] first\ncontinuation one\r\n[B] second\rcontinuation two",
        );
        let mut options = framed(r"^\[(?<kind>[A-Z])\] ");
        options.start_rownum = Some(10);
        options.set_batch_row_size(Some(1));

        let batches = collect(&source, options);
        assert_eq!(batches.len(), 2);
        assert_eq!(
            bodies(&batches),
            [
                b"[A] first\ncontinuation one".to_vec(),
                b"[B] second\ncontinuation two".to_vec(),
            ]
        );
        assert_eq!(rownums(&batches), [10, 12]);
    }
}
