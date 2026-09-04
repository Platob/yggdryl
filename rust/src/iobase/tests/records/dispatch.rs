use super::*;

#[test]
fn generic_write_entry_points_compose_the_three_typed_shapes() {
    let mut handle = handle("generic-write-mode.arrows");
    let options = handle
        .record_options()
        .unwrap()
        .with_field(schema())
        .with_batch_row_size(1);

    handle
        .write_arrow_reader(reader(), IOMode::Overwrite, &options)
        .unwrap();
    handle
        .write_arrow_batch(rows_batch(&[3]), IOMode::Append, &options)
        .unwrap();
    handle
        .write_records(
            [
                NativeRow {
                    id: 2,
                    symbol: Some("updated"),
                },
                NativeRow {
                    id: 4,
                    symbol: Some("AMD"),
                },
            ],
            IOMode::Merge,
            &options.clone().with_merge_by_names(["id"]),
        )
        .unwrap();

    assert_eq!(rows(&handle, &options), 4);
}

#[test]
fn generic_write_entry_points_preserve_commit_cadence() {
    let pulls = Arc::new(AtomicUsize::new(0));
    let mut reader_handle =
        PublicationProbe::new("generic-reader-commits.arrows", Arc::clone(&pulls));
    let options = reader_handle
        .record_options()
        .unwrap()
        .with_field(schema())
        .with_commit_row_size(1);
    reader_handle
        .write_arrow_reader(reader(), IOMode::Overwrite, &options)
        .unwrap();
    assert_eq!(reader_handle.publications.load(Ordering::SeqCst), 2);

    let mut batch_handle =
        PublicationProbe::new("generic-batch-commits.arrows", Arc::clone(&pulls));
    batch_handle
        .write_arrow_batch(batch(), IOMode::Overwrite, &options)
        .unwrap();
    assert_eq!(batch_handle.publications.load(Ordering::SeqCst), 2);

    let mut record_handle = PublicationProbe::new("generic-row-commits.arrows", pulls);
    record_handle
        .write_records(
            [
                NativeRow {
                    id: 1,
                    symbol: Some("AAPL"),
                },
                NativeRow {
                    id: 2,
                    symbol: Some("MSFT"),
                },
            ],
            IOMode::Overwrite,
            &options,
        )
        .unwrap();
    assert_eq!(record_handle.publications.load(Ordering::SeqCst), 2);
}

/// A media may preserve a same-shape optimization while still converging
/// on the reader primitives. The generic entry points must select that
/// authoritative adapter rather than rebuilding its input themselves.
struct TypedDispatchProbe {
    handle: Buffer,
    reader_calls: [usize; 3],
    batch_calls: [usize; 3],
    record_calls: [usize; 3],
}

impl TypedDispatchProbe {
    fn new() -> Self {
        Self {
            handle: handle("typed-dispatch-probe.arrows"),
            reader_calls: [0; 3],
            batch_calls: [0; 3],
            record_calls: [0; 3],
        }
    }
}

impl IOMedia for TypedDispatchProbe {
    fn as_io_base(&self) -> &dyn IOBase {
        self
    }

    fn as_io_base_mut(&mut self) -> &mut dyn IOBase {
        self
    }

    fn overwrite_arrow_reader(
        &mut self,
        _batches: BatchReader,
        _options: &RecordOptions,
    ) -> crate::Result<()> {
        self.reader_calls[0] += 1;
        Ok(())
    }

    fn append_arrow_reader(
        &mut self,
        _batches: BatchReader,
        _options: &RecordOptions,
    ) -> crate::Result<()> {
        self.reader_calls[1] += 1;
        Ok(())
    }

    fn merge_arrow_reader(
        &mut self,
        _batches: BatchReader,
        _options: &RecordOptions,
    ) -> crate::Result<()> {
        self.reader_calls[2] += 1;
        Ok(())
    }

    fn overwrite_arrow_batch(
        &mut self,
        _batch: RecordBatch,
        _options: &RecordOptions,
    ) -> crate::Result<()> {
        self.batch_calls[0] += 1;
        Ok(())
    }

    fn append_arrow_batch(
        &mut self,
        _batch: RecordBatch,
        _options: &RecordOptions,
    ) -> crate::Result<()> {
        self.batch_calls[1] += 1;
        Ok(())
    }

    fn merge_arrow_batch(
        &mut self,
        _batch: RecordBatch,
        _options: &RecordOptions,
    ) -> crate::Result<()> {
        self.batch_calls[2] += 1;
        Ok(())
    }

    fn overwrite_records<I, R>(
        &mut self,
        _records: I,
        _options: &RecordOptions,
    ) -> crate::Result<()>
    where
        Self: Sized,
        I: IntoIterator<Item = R>,
        I::IntoIter: Send + 'static,
        R: TryInto<Scalar>,
        R::Error: Into<Error>,
    {
        self.record_calls[0] += 1;
        Ok(())
    }

    fn append_records<I, R>(&mut self, _records: I, _options: &RecordOptions) -> crate::Result<()>
    where
        Self: Sized,
        I: IntoIterator<Item = R>,
        I::IntoIter: Send + 'static,
        R: TryInto<Scalar>,
        R::Error: Into<Error>,
    {
        self.record_calls[1] += 1;
        Ok(())
    }

    fn merge_records<I, R>(&mut self, _records: I, _options: &RecordOptions) -> crate::Result<()>
    where
        Self: Sized,
        I: IntoIterator<Item = R>,
        I::IntoIter: Send + 'static,
        R: TryInto<Scalar>,
        R::Error: Into<Error>,
    {
        self.record_calls[2] += 1;
        Ok(())
    }
}

impl IOBase for TypedDispatchProbe {
    crate::delegate_iobase!(handle);
}

#[test]
fn generic_writes_select_the_same_shape_for_every_mode() {
    let mut probe = TypedDispatchProbe::new();
    let plain = probe.record_options().unwrap().with_field(schema());

    for mode in IOMode::WRITE {
        let options = if mode == IOMode::Merge {
            plain.clone().with_merge_by_names(["id"])
        } else {
            plain.clone()
        };
        probe.write_arrow_reader(reader(), mode, &options).unwrap();
        probe.write_arrow_batch(batch(), mode, &options).unwrap();
        probe
            .write_records(std::iter::empty::<NativeRow>(), mode, &options)
            .unwrap();
    }

    assert_eq!(probe.reader_calls, [1, 1, 1]);
    assert_eq!(probe.batch_calls, [1, 1, 1]);
    assert_eq!(probe.record_calls, [1, 1, 1]);
}

#[test]
fn generic_arrow_writes_remain_object_safe() {
    let mut probe = TypedDispatchProbe::new();
    let options = probe.record_options().unwrap().with_field(schema());
    let media: &mut dyn IOMedia = &mut probe;

    media
        .write_arrow_reader(reader(), IOMode::Overwrite, &options)
        .unwrap();
    media
        .write_arrow_batch(batch(), IOMode::Append, &options)
        .unwrap();

    assert_eq!(probe.reader_calls, [1, 0, 0]);
    assert_eq!(probe.batch_calls, [0, 1, 0]);
}

#[test]
fn generic_writes_validate_mode_before_touching_input() {
    let mut handle = handle("generic-write-mode-validation.arrows");
    let options = handle.record_options().unwrap().with_field(schema());

    // The required mode is validated before a one-shot source is pulled;
    // a match key never silently turns overwrite into merge.
    let pulls = Arc::new(AtomicUsize::new(0));
    let source = counted_source(Arc::clone(&pulls), [Ok(rows_batch(&[5]))]);
    let error = handle
        .write_arrow_reader(
            source,
            IOMode::Overwrite,
            &options.clone().with_merge_by_names(["id"]),
        )
        .unwrap_err();
    assert!(error.to_string().contains("write mode overwrite"));
    assert_eq!(pulls.load(Ordering::SeqCst), 0);

    // A held batch is already materialized, but invalid intent still does
    // not reach the destination or silently select merge.
    let before = handle.as_slice().to_vec();
    let error = handle
        .write_arrow_batch(
            rows_batch(&[6]),
            IOMode::Overwrite,
            &options.clone().with_merge_by_names(["id"]),
        )
        .unwrap_err();
    assert!(error.to_string().contains("write mode overwrite"));
    assert_eq!(handle.as_slice(), before.as_slice());

    struct CountedIntoRows(Arc<AtomicUsize>);

    impl IntoIterator for CountedIntoRows {
        type Item = NativeRow;
        type IntoIter = std::iter::Empty<NativeRow>;

        fn into_iter(self) -> Self::IntoIter {
            self.0.fetch_add(1, Ordering::SeqCst);
            std::iter::empty()
        }
    }

    // Mode validation also wins over the missing-field error and happens
    // before even constructing a native row iterator.
    let into_iters = Arc::new(AtomicUsize::new(0));
    let untyped = handle.record_options().unwrap().with_merge_by_names(["id"]);
    let error = handle
        .write_records(
            CountedIntoRows(Arc::clone(&into_iters)),
            IOMode::Overwrite,
            &untyped,
        )
        .unwrap_err();
    assert!(error.to_string().contains("write mode overwrite"));
    assert_eq!(into_iters.load(Ordering::SeqCst), 0);
}
