//! `rust/src/object/file.rs`: one object as a byte leaf, and what each
//! operation over it costs in round trips.

mod accounting {
    use crate::mod_::{BUCKET, file, file_with, options, payload, store};
    use yggdryl::internals::object_file::upload_from;
    use yggdryl::{IOBase, IOKind};

    /// A source that answers in pieces smaller than it is asked for, and
    /// remembers the most it was ever asked for at once - which is the buffer
    /// the upload holds, and so the memory it costs.
    struct Metered<'bytes> {
        bytes: &'bytes [u8],
        position: usize,
        largest_ask: usize,
        reads: usize,
    }

    impl<'bytes> Metered<'bytes> {
        fn over(bytes: &'bytes [u8]) -> Self {
            Self {
                bytes,
                position: 0,
                largest_ask: 0,
                reads: 0,
            }
        }
    }

    impl std::io::Read for Metered<'_> {
        fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
            self.largest_ask = self.largest_ask.max(buffer.len());
            self.reads += 1;
            let length = buffer
                .len()
                .min(self.bytes.len() - self.position)
                .min(100 * 1024);
            buffer[..length].copy_from_slice(&self.bytes[self.position..self.position + length]);
            self.position += length;
            Ok(length)
        }
    }

    #[test]
    fn a_whole_read_is_one_request_and_so_is_a_ranged_one() {
        let store = store();
        let bytes = payload(4096);
        store.put(BUCKET, "lake/part.parquet", &bytes);
        let handle = file(&store, "lake/part.parquet");

        store.clear_requests();
        assert_eq!(handle.read_all_bytes().expect("the object"), bytes);
        assert_eq!(store.request_count(), 1, "a whole read is one GET");

        // A ranged read must not ask for the size first: the range answers it.
        store.clear_requests();
        let footer = handle.read_range_bytes(4088, 8).expect("the footer");
        assert_eq!(footer, &bytes[4088..]);
        assert_eq!(store.request_count(), 1, "a ranged read is one GET");
        // And it transferred the range, not the object.
        let recorded = store.requests();
        assert_eq!(recorded[0].method, "GET");
        let range = recorded[0]
            .headers
            .iter()
            .find(|(name, _)| name == "range")
            .map(|(_, value)| value.clone());
        assert_eq!(range.as_deref(), Some("bytes=4088-4095"));

        // A positional read is the same one request.
        store.clear_requests();
        let mut window = [0_u8; 16];
        assert_eq!(handle.pread(100, &mut window).expect("a window"), 16);
        assert_eq!(window, bytes[100..116]);
        assert_eq!(store.request_count(), 1);
    }

    #[test]
    fn a_full_stream_drain_is_one_request_not_one_per_chunk() {
        let store = store();
        let bytes = payload(64 * 1024);
        store.put(BUCKET, "lake/part.parquet", &bytes);
        let handle = file(&store, "lake/part.parquet");

        store.clear_requests();
        let chunks: Vec<Vec<u8>> = handle
            .pstream_bytes(0, 4096)
            .expect("a stream")
            .collect::<yggdryl::Result<Vec<_>>>()
            .expect("every chunk");
        assert_eq!(chunks.len(), 16, "the payload arrives in bounded pieces");
        assert_eq!(chunks.concat(), bytes);
        assert_eq!(
            store.request_count(),
            1,
            "sixteen chunks came out of one GET"
        );

        // A digest streams the same way, so it is one request over any size.
        store.clear_requests();
        let digest = handle
            .read_digest(yggdryl::DigestAlgorithm::Xxh3)
            .expect("a digest");
        assert_eq!(digest, yggdryl::DigestAlgorithm::Xxh3.digest(&bytes));
        assert_eq!(store.request_count(), 1, "a digest is one GET");
    }

    #[test]
    fn a_whole_write_is_one_request_and_an_append_is_two() {
        let store = store();
        let mut handle = file(&store, "lake/part.parquet");

        store.clear_requests();
        handle.write_all_bytes(b"AAPL,187.23\n").expect("a write");
        assert_eq!(
            store.request_count(),
            1,
            "a whole write loads nothing first"
        );
        assert_eq!(store.requests()[0].method, "PUT");

        // What a closed handle publishes belongs to the store, and the handle
        // lets go of it: appending after that re-reads, because a value written
        // through this handle a moment ago is not evidence about what is there
        // now, and holding the payload would keep the whole object in memory for
        // as long as the handle lived.
        store.clear_requests();
        let offset = handle.append_bytes(b"MSFT,410.10\n").expect("an append");
        assert_eq!(offset, 12);
        assert_eq!(
            store.request_count(),
            2,
            "a closed handle re-reads before it appends"
        );
        assert_eq!(
            store.get(BUCKET, "lake/part.parquet").expect("the object"),
            b"AAPL,187.23\nMSFT,410.10\n"
        );

        // An *open* handle is a scope that asked for a coherent view, so it keeps
        // what it wrote and the append is the upload alone.
        let mut open = file(&store, "lake/warm.parquet");
        open.open().expect("an open");
        store.clear_requests();
        open.write_all_bytes(b"AAPL,187.23\n").expect("a write");
        open.append_bytes(b"MSFT,410.10\n").expect("an append");
        assert_eq!(
            store.request_count(),
            2,
            "one PUT each, and nothing read back"
        );
        assert!(
            store
                .requests()
                .iter()
                .all(|request| request.method == "PUT"),
            "an open handle appends to what it holds"
        );
        open.close().expect("a close");
        assert_eq!(
            store.get(BUCKET, "lake/warm.parquet").expect("the object"),
            b"AAPL,187.23\nMSFT,410.10\n"
        );

        // A fresh handle has to read what is there first, because S3 replaces
        // whole objects and has no append of its own.
        let mut cold = file(&store, "lake/part.parquet");
        store.clear_requests();
        cold.append_bytes(b"GOOG,180.00\n").expect("an append");
        assert_eq!(
            store.request_count(),
            2,
            "a cold append is one GET and one PUT"
        );
        assert_eq!(
            store.get(BUCKET, "lake/part.parquet").expect("the object"),
            b"AAPL,187.23\nMSFT,410.10\nGOOG,180.00\n"
        );

        // Positional writes through a fresh handle load once and publish once,
        // however many of them there are.
        let mut staged = file(&store, "lake/part.parquet");
        store.clear_requests();
        staged.pwrite(0, b"GOOG").expect("a staged write");
        staged.pwrite(4, b"!").expect("a staged write");
        assert_eq!(store.request_count(), 1, "one load, and nothing published");
        staged.flush().expect("a publish");
        assert_eq!(store.request_count(), 2, "the flush is the second request");
    }

    #[test]
    fn a_removal_is_one_request_and_absence_is_a_success() {
        let store = store();
        store.put(BUCKET, "lake/part.parquet", b"PAR1");
        let mut handle = file(&store, "lake/part.parquet");

        store.clear_requests();
        handle.remove(false).expect("a removal");
        assert_eq!(store.request_count(), 1, "one DELETE, with no probe first");
        assert_eq!(store.requests()[0].method, "DELETE");

        // Removing again succeeds without asking whether there is anything there.
        store.clear_requests();
        handle.remove(false).expect("a second removal");
        assert_eq!(store.request_count(), 1);
        assert!(store.get(BUCKET, "lake/part.parquet").is_none());
    }

    #[test]
    fn an_opened_object_stops_asking_for_its_metadata() {
        let store = store();
        store.put(BUCKET, "lake/part.parquet", &payload(1024));
        let mut handle = file(&store, "lake/part.parquet");

        // Closed, every size question is a fresh HEAD: that is the contract.
        store.clear_requests();
        assert_eq!(handle.size(), 1024);
        assert_eq!(handle.size(), 1024);
        assert_eq!(store.request_count(), 2, "a closed handle keeps nothing");

        store.clear_requests();
        handle.open().expect("an open");
        assert_eq!(
            store.request_count(),
            1,
            "opening is one HEAD, not a download"
        );
        assert_eq!(store.requests()[0].method, "HEAD");

        store.clear_requests();
        for _ in 0..5 {
            assert_eq!(handle.size(), 1024);
            assert_eq!(handle.kind(), IOKind::File);
            assert!(!handle.is_empty());
        }
        assert_eq!(store.request_count(), 0, "the open scope answers them all");

        // Closing drops it, so the next question is fresh again.
        handle.close().expect("a close");
        store.clear_requests();
        assert_eq!(handle.size(), 1024);
        assert_eq!(store.request_count(), 1);
    }

    #[test]
    fn a_read_teaches_an_open_handle_the_size_it_did_not_ask_for() {
        let store = store();
        store.put(BUCKET, "lake/part.parquet", &payload(2048));
        let mut handle = file(&store, "lake/part.parquet");
        handle.open().expect("an open");

        store.clear_requests();
        let window = handle.read_range_bytes(0, 16).expect("a window");
        assert_eq!(window.len(), 16);
        // The ranged answer stated the total, so the size is already known.
        assert_eq!(handle.size(), 2048);
        assert_eq!(store.request_count(), 1, "the read was the only request");
    }

    #[test]
    fn a_large_write_uploads_in_parts_and_a_small_one_does_not() {
        let store = store();
        // A threshold far below the default, so the fixture stays small while the
        // shape of the upload is the real one. The part size is clamped up to
        // S3's own 5 MiB floor, so this payload is one part.
        let bounded = || {
            options(&store)
                .with_multipart_threshold(512 * 1024)
                .with_part_size(5 * 1024 * 1024)
        };

        let mut small = file_with("lake/small.bin", bounded());
        store.clear_requests();
        small
            .write_all_bytes(&payload(1024))
            .expect("a small write");
        assert_eq!(
            store.request_count(),
            1,
            "below the threshold it is one PUT"
        );

        let bytes = payload(600 * 1024);
        let mut big = file_with("lake/big.bin", bounded());
        store.clear_requests();
        big.write_all_bytes(&bytes).expect("a large write");
        assert_eq!(
            store.request_count(),
            3,
            "a multipart upload is its parts plus two"
        );
        assert_eq!(
            store.get(BUCKET, "lake/big.bin").expect("the object"),
            bytes
        );
        assert_eq!(
            store.open_uploads(),
            0,
            "the upload was completed, not left open"
        );
    }

    /// A streamed upload holds one part of its source at a time: above the
    /// threshold each part is read and sent before the next is read, and the
    /// source is never asked for more than a part; below it the source is read
    /// once. A source that ends short is refused with nothing stored.
    #[test]
    fn a_streamed_upload_reads_its_source_one_part_at_a_time() {
        let store = store();
        let part = 5 * 1024 * 1024;
        let bounded = || {
            options(&store)
                .with_multipart_threshold(512 * 1024)
                .with_part_size(part as u64)
        };

        // Six mebibytes over five-mebibyte parts: two parts, plus the create
        // and the complete. The largest read is one part, never the object.
        let bytes = payload(6 * 1024 * 1024);
        let mut big = file_with("lake/streamed.bin", bounded());
        let mut source = Metered::over(&bytes);
        store.clear_requests();
        upload_from(&mut big, &mut source, bytes.len() as u64).expect("a streamed upload");
        assert_eq!(
            store.request_count(),
            4,
            "a multipart upload is its parts plus two"
        );
        assert_eq!(
            store.get(BUCKET, "lake/streamed.bin").expect("the object"),
            bytes
        );
        assert_eq!(
            source.largest_ask, part,
            "the source is asked for one part at a time, never the whole"
        );
        assert!(
            source.reads > 60,
            "the fill loop takes what the source gives"
        );
        assert_eq!(store.open_uploads(), 0);

        // Below the threshold: one PUT of the source read into one buffer.
        let small = payload(1024);
        let mut file = file_with("lake/streamed-small.bin", bounded());
        let mut source = Metered::over(&small);
        store.clear_requests();
        upload_from(&mut file, &mut source, small.len() as u64).expect("a small streamed upload");
        assert_eq!(
            store.request_count(),
            1,
            "below the threshold it is one PUT"
        );
        assert_eq!(store.get(BUCKET, "lake/streamed-small.bin"), Some(small));

        // A source that ends before the length it declared is refused: the
        // upload is aborted, nothing is stored under the key, and dropping the
        // handle retries nothing.
        let short = payload(300 * 1024);
        let mut file = file_with("lake/short.bin", bounded());
        let mut source = Metered::over(&short);
        store.clear_requests();
        let error =
            upload_from(&mut file, &mut source, 600 * 1024).expect_err("a short source is refused");
        assert!(
            error.to_string().contains("expected 614400 bytes"),
            "{error}"
        );
        assert_eq!(store.get(BUCKET, "lake/short.bin"), None);
        assert_eq!(store.open_uploads(), 0, "the abandoned upload was aborted");
        let requests = store.request_count();
        drop(file);
        assert_eq!(
            store.request_count(),
            requests,
            "nothing is retried on drop"
        );
        assert_eq!(store.get(BUCKET, "lake/short.bin"), None);
    }

    #[test]
    fn opening_an_object_twice_asks_once() {
        let store = store();
        store.put(BUCKET, "lake/part.parquet", &payload(4096));
        let mut handle = file(&store, "lake/part.parquet");

        store.clear_requests();
        handle.open().expect("an open");
        handle.open().expect("an open");
        handle.open().expect("an open");
        assert_eq!(
            store.request_count(),
            1,
            "opening what is already open asks nothing"
        );
        assert_eq!(handle.size(), 4096);
        assert_eq!(store.request_count(), 1, "and the size is already known");
    }

    #[test]
    fn a_ranged_digest_asks_for_the_range_and_not_the_tail() {
        let store = store();
        store.put(BUCKET, "lake/part.parquet", &payload(256 * 1024));
        let handle = file(&store, "lake/part.parquet");

        store.clear_requests();
        let digest = handle
            .read_range_digest(0, 16, yggdryl::DigestAlgorithm::Xxh3)
            .expect("a digest");
        assert_eq!(digest, yggdryl::DigestAlgorithm::Xxh3.digest(&payload(16)));
        assert_eq!(store.request_count(), 1, "one GET");
        let range = store.requests()[0]
            .headers
            .iter()
            .find(|(name, _)| name == "range")
            .map(|(_, value)| value.clone());
        assert_eq!(
            range.as_deref(),
            Some("bytes=0-15"),
            "the window asked for is the window wanted"
        );
    }
}

mod protocol {

    use yggdryl::{IOBase, IOKind};

    use crate::mod_::{BUCKET, file, file_with, options, payload, store};

    #[test]
    fn absence_is_emptiness_on_every_read_and_a_success_on_every_delete() {
        let store = store();
        let mut handle = file(&store, "lake/absent.parquet");

        assert!(handle.read_all_bytes().expect("an empty read").is_empty());
        assert_eq!(handle.size(), 0);
        assert!(handle.is_empty());
        assert_eq!(handle.kind(), IOKind::Unknown);
        let mut window = [0_u8; 8];
        assert_eq!(handle.pread(0, &mut window).expect("an empty read"), 0);
        assert_eq!(handle.pread(4096, &mut window).expect("an empty read"), 0);
        assert!(
            handle
                .pstream_bytes(0, 1024)
                .expect("a stream")
                .next()
                .is_none()
        );
        handle.remove(true).expect("a removal of nothing");

        // A digest of nothing is the algorithm's empty-input value, not an error.
        assert_eq!(
            handle
                .read_digest(yggdryl::DigestAlgorithm::Xxh3)
                .expect("a digest")
                .as_u64(),
            Some(yggdryl::xxhash::xxh3(b"")),
        );
    }

    #[test]
    fn a_range_past_the_end_reads_nothing_and_a_straddling_one_is_short() {
        let store = store();
        store.put(BUCKET, "lake/part.parquet", &payload(100));
        let handle = file(&store, "lake/part.parquet");

        let mut window = [0_u8; 32];
        assert_eq!(handle.pread(90, &mut window).expect("a short read"), 10);
        assert_eq!(handle.pread(100, &mut window).expect("an empty read"), 0);
        assert_eq!(handle.pread(1000, &mut window).expect("an empty read"), 0);
        // A ranged read clamps to what is there, exactly as the trait says.
        assert_eq!(handle.read_range_bytes(95, 50).expect("a range").len(), 5);
    }

    #[test]
    fn the_media_type_comes_from_the_key_without_asking_the_store() {
        let store = store();
        let parquet = file(&store, "lake/part.parquet");
        let arrow = file(&store, "lake/part.arrows");
        let nameless = file(&store, "lake/part");

        assert_eq!(parquet.media_type().base(), &yggdryl::MimeType::PARQUET);
        assert!(parquet.is_tabular());
        assert_eq!(arrow.media_type().base(), &yggdryl::MimeType::ARROW_STREAM);
        assert_eq!(nameless.media_type().base(), &yggdryl::MimeType::FILE);
        assert_eq!(store.request_count(), 0, "a name is free evidence");

        // A declared type overrides the name, still without a request.
        let mut declared = file(&store, "lake/part.parquet");
        declared.set_media_type(yggdryl::MediaType::from(yggdryl::MimeType::CSV));
        assert_eq!(declared.media_type().base(), &yggdryl::MimeType::CSV);
        assert_eq!(store.request_count(), 0);
    }

    #[test]
    fn an_empty_value_is_one_put_however_low_the_multipart_threshold() {
        let store = store();
        let options = options(&store).with_multipart_threshold(0);
        let mut handle = file_with("lake/empty.bin", options);

        store.clear_requests();
        handle.write_all_bytes(b"").expect("a write");
        assert_eq!(
            store.request_count(),
            1,
            "a multipart upload of no parts is not something S3 completes"
        );
        assert_eq!(store.requests()[0].method, "PUT");
        assert_eq!(store.open_uploads(), 0, "and nothing was left open");
        assert_eq!(store.get(BUCKET, "lake/empty.bin"), Some(Vec::new()));
    }

    #[test]
    fn a_transfer_cut_part_way_through_resumes_from_where_it_stopped() {
        let store = store();
        let payload = payload(64 * 1024);
        store.put(BUCKET, "lake/part.bin", &payload);
        let handle = file(&store, "lake/part.bin");

        // The next body stops after 8 KiB with its declared length unchanged,
        // which is what a severed connection looks like from the inside.
        store.cut_next_body(8 * 1024, 1);
        store.clear_requests();
        let streamed: Vec<u8> = handle
            .pstream_bytes(0, 4096)
            .expect("a stream")
            .flat_map(|chunk| chunk.expect("bytes"))
            .collect();
        assert_eq!(
            streamed, payload,
            "the caller sees one uninterrupted stream"
        );

        let ranges: Vec<String> = store
            .requests()
            .iter()
            .filter_map(|request| {
                request
                    .headers
                    .iter()
                    .find(|(name, _)| name == "range")
                    .map(|(_, value)| value.clone())
            })
            .collect();
        assert_eq!(
            ranges,
            vec!["bytes=8192-".to_owned()],
            "the resumed request asks for the rest, not for the whole object again"
        );
        assert_eq!(store.request_count(), 2, "the open, and the one resume");
    }

    #[test]
    fn a_transfer_that_cannot_deliver_a_byte_stops_rather_than_looping() {
        let store = store();
        store.put(BUCKET, "lake/part.bin", &payload(64 * 1024));
        let handle = file_with("lake/part.bin", options(&store).with_max_attempts(3));

        // Every body is cut before a single byte arrives, so nothing ever
        // progresses and the consecutive-failure budget is what ends it.
        store.cut_next_body(0, 100);
        store.clear_requests();
        let outcome: yggdryl::Result<Vec<u8>> = handle
            .pstream_bytes(0, 4096)
            .expect("a stream")
            .collect::<yggdryl::Result<Vec<_>>>()
            .map(|chunks| chunks.concat());
        assert!(outcome.is_err(), "a stream that never moves is a failure");
        assert!(
            store.request_count() <= 3,
            "bounded by the attempt limit, not by the object: {}",
            store.request_count()
        );
    }
}
