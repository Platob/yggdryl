//! `rust/src/s3/file.rs`: one object as a byte leaf, and what each
//! operation over it costs in round trips.

mod accounting {
    use crate::mod_::{BUCKET, file, file_with, options, payload, store};
    use yggdryl::internals::s3_file::upload_from;
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

/// `IOBase::create_bytes` on an object: the exclusive condition each store
/// reads, on the request that decides the object, and its refusal as the
/// conflict that leaves the stored object as it was.
mod create {
    use crate::mod_::{BUCKET, file, file_on, file_on_with, options_for, payload, store};
    use crate::server::Recorded;
    use yggdryl::s3::Provider;
    use yggdryl::{Error, IOBase};

    const EVERY: [Provider; 3] = [Provider::Aws, Provider::Google, Provider::Azure];

    fn header<'a>(request: &'a Recorded, name: &str) -> Option<&'a str> {
        request
            .headers
            .iter()
            .find(|(held, _)| held == name)
            .map(|(_, value)| value.as_str())
    }

    fn query<'a>(request: &'a Recorded, name: &str) -> Option<&'a str> {
        request
            .query
            .iter()
            .find(|(held, _)| held == name)
            .map(|(_, value)| value.as_str())
    }

    /// Whether `request` carries the exclusive condition `provider` reads,
    /// and nothing of the other stores' spelling.
    fn conditioned(provider: Provider, request: &Recorded) -> bool {
        match provider {
            Provider::Aws | Provider::Azure => {
                header(request, "if-none-match") == Some("*")
                    && query(request, "ifGenerationMatch").is_none()
            }
            Provider::Google => {
                query(request, "ifGenerationMatch") == Some("0")
                    && header(request, "if-none-match").is_none()
            }
        }
    }

    #[test]
    fn a_create_is_one_request_carrying_the_condition_its_store_reads() {
        let store = store();
        for provider in EVERY {
            let key = format!("lake/{}-claim.json", provider.service());
            store.clear_requests();
            file_on(&store, provider, &key)
                .create_bytes(b"{\"version\":1}")
                .expect("nothing at the key");
            let sent = store.requests();
            assert_eq!(sent.len(), 1, "one upload on {provider}");
            let method = if provider == Provider::Google {
                "POST"
            } else {
                "PUT"
            };
            assert_eq!(sent[0].method, method, "{provider}");
            assert!(conditioned(provider, &sent[0]), "{provider}: {:?}", sent[0]);
            assert_eq!(
                store.get(BUCKET, &key).as_deref(),
                Some(&b"{\"version\":1}"[..]),
                "{provider}"
            );

            // The second creator loses with one request, and the object stands.
            store.clear_requests();
            let error = file_on(&store, provider, &key)
                .create_bytes(b"{\"version\":2}")
                .expect_err("an object is at the key");
            assert!(
                matches!(error, Error::Conflict { .. }),
                "{provider}: {error}"
            );
            assert!(error.to_string().contains(&key), "{provider}: {error}");
            let sent = store.requests();
            assert_eq!(sent.len(), 1, "one refused upload on {provider}");
            let refused = if provider == Provider::Azure {
                409
            } else {
                412
            };
            assert_eq!(sent[0].status, refused, "{provider}");
            assert_eq!(
                store.get(BUCKET, &key).as_deref(),
                Some(&b"{\"version\":1}"[..]),
                "{provider}"
            );
        }
    }

    #[test]
    fn a_chunked_create_carries_the_condition_on_the_request_that_decides_the_object() {
        let store = store();
        let bytes = payload(6 * 1024 * 1024);
        for provider in EVERY {
            let key = format!("lake/{}-big.bin", provider.service());
            let options = || {
                options_for(&store, provider)
                    .with_part_size(5 * 1024 * 1024)
                    .with_multipart_threshold(1024 * 1024)
            };
            store.clear_requests();
            file_on_with(provider, &key, options())
                .create_bytes(&bytes)
                .expect("nothing at the key");
            let sent = store.requests();
            // S3 completes the parts, Azure commits the block list, and
            // Google's session holds the condition its initiating POST named.
            let decides = |request: &Recorded| match provider {
                Provider::Aws => request.method == "POST" && query(request, "uploadId").is_some(),
                Provider::Azure => query(request, "comp") == Some("blocklist"),
                Provider::Google => query(request, "uploadType") == Some("resumable"),
            };
            for request in &sent {
                assert_eq!(
                    conditioned(provider, request),
                    decides(request),
                    "{provider}: only the deciding request is conditioned: {request:?}"
                );
            }
            assert_eq!(
                sent.iter().filter(|request| decides(request)).count(),
                1,
                "{provider}"
            );
            assert_eq!(
                store.get(BUCKET, &key).as_deref(),
                Some(&bytes[..]),
                "{provider}"
            );

            // A losing chunked create stores nothing and leaves no upload open.
            let error = file_on_with(provider, &key, options())
                .create_bytes(&payload(2 * 1024 * 1024))
                .expect_err("an object is at the key");
            assert!(error.is_conflict(), "{provider}: {error}");
            assert_eq!(
                store.get(BUCKET, &key).as_deref(),
                Some(&bytes[..]),
                "{provider}"
            );
            assert_eq!(store.open_uploads(), 0, "{provider}");
        }
    }

    /// A create goes again only after an attempt the store cannot have acted
    /// on: a throttle is retried, a server error is not, so a second attempt
    /// never reads the create's own object as the conflict.
    #[test]
    fn a_create_is_retried_after_a_throttle_and_never_after_a_server_error() {
        let store = store();
        store.fail_next(503, "SlowDown", 1);
        store.clear_requests();
        file(&store, "lake/throttled.json")
            .create_bytes(b"one")
            .expect("the throttle is retried");
        assert_eq!(store.request_count(), 2);

        store.fail_next(500, "InternalError", 1);
        store.clear_requests();
        let error = file(&store, "lake/failed.json")
            .create_bytes(b"one")
            .expect_err("a server error ends the create");
        assert!(!error.is_conflict(), "{error}");
        assert_eq!(store.request_count(), 1);

        // A replacing write retries the same server error, as it always did.
        store.fail_next(500, "InternalError", 1);
        store.clear_requests();
        file(&store, "lake/failed.json")
            .write_all_bytes(b"one")
            .expect("a replacing write is retried");
        assert_eq!(store.request_count(), 2);
    }

    /// An open handle that created keeps what it created; one that lost
    /// forgets what it believed of the key and reads the stored value.
    #[test]
    fn a_handle_keeps_what_it_created_and_forgets_what_a_conflict_contradicts() {
        let store = store();
        let mut handle = file(&store, "lake/kept.json");
        handle.open().unwrap();
        assert_eq!(handle.size(), 0, "nothing is there yet");
        handle.create_bytes(b"created").unwrap();
        store.clear_requests();
        assert_eq!(handle.read_all_bytes().unwrap(), b"created");
        assert_eq!(handle.size(), 7);
        assert_eq!(
            store.request_count(),
            0,
            "an open handle holds what it created"
        );

        store.put(BUCKET, "lake/taken.json", b"theirs");
        let mut loser = file(&store, "lake/taken.json");
        loser.open().unwrap();
        assert_eq!(loser.size(), 6);
        assert!(loser.create_bytes(b"mine").unwrap_err().is_conflict());
        assert_eq!(loser.read_all_bytes().unwrap(), b"theirs");
    }

    /// A `PutObject` Amazon S3 answers `409 ConditionalRequestConflict` met
    /// another conditional write in flight on its key, and the store did not
    /// act: the create is sent again and lands, never told an object is there.
    #[test]
    fn a_create_that_raced_another_conditional_write_is_sent_again() {
        let store = store();
        store.fail_next(409, "ConditionalRequestConflict", 1);
        store.clear_requests();
        file(&store, "lake/raced.json")
            .create_bytes(b"one")
            .expect("the race is sent again");
        let sent = store.requests();
        let seen: Vec<(&str, u16)> = sent
            .iter()
            .map(|request| (request.method.as_str(), request.status))
            .collect();
        assert_eq!(seen, [("PUT", 409), ("PUT", 200)]);
        assert!(
            sent.iter()
                .all(|request| header(request, "if-none-match") == Some("*"))
        );
        assert_eq!(
            store.get(BUCKET, "lake/raced.json").as_deref(),
            Some(&b"one"[..])
        );
    }

    /// The retry budget bounds the races a create is sent again after, and
    /// the last one is what the store said - never a conflict, since no
    /// object is at the key.
    #[test]
    fn a_create_that_keeps_racing_is_the_stores_own_refusal() {
        let store = store();
        store.fail_next(409, "ConditionalRequestConflict", 3);
        store.clear_requests();
        let error = file(&store, "lake/contended.json")
            .create_bytes(b"one")
            .expect_err("every attempt raced");
        assert!(
            matches!(
                &error,
                Error::Remote { status: 409, code, operation: "PutObject", .. }
                    if code == "ConditionalRequestConflict"
            ),
            "{error:?}"
        );
        assert!(!error.is_conflict(), "{error}");
        assert_eq!(store.request_count(), 3, "the default three attempts");
        assert_eq!(store.get(BUCKET, "lake/contended.json"), None);
    }

    /// A completion that raced spends its upload - Amazon S3 has the whole
    /// upload initiated again - so the create abandons it and uploads the
    /// value once more as a new one, the condition on its completion again.
    #[test]
    fn a_chunked_create_that_raced_is_uploaded_again_as_a_new_upload() {
        let store = store();
        let bytes = payload(6 * 1024 * 1024);
        let options = options_for(&store, Provider::Aws)
            .with_part_size(5 * 1024 * 1024)
            .with_multipart_threshold(1024 * 1024);
        // The initiation and the two parts land; the completion races.
        store.fail_after(3, 409, "ConditionalRequestConflict", 1);
        store.clear_requests();
        file_on_with(Provider::Aws, "lake/raced.bin", options)
            .create_bytes(&bytes)
            .expect("the upload is sent again");
        let sent = store.requests();
        let step = |request: &Recorded| match request.method.as_str() {
            "POST" if query(request, "uploads").is_some() => "initiate",
            "POST" => "complete",
            "PUT" => "part",
            "DELETE" => "abort",
            other => panic!("an unexpected {other}"),
        };
        let steps: Vec<&str> = sent.iter().map(step).collect();
        assert_eq!(
            steps,
            [
                "initiate", "part", "part", "complete", "abort", "initiate", "part", "part",
                "complete"
            ]
        );
        let completions: Vec<&Recorded> = sent
            .iter()
            .filter(|request| step(request) == "complete")
            .collect();
        assert_eq!(completions[0].status, 409);
        assert_eq!(completions[1].status, 200);
        assert_ne!(
            query(completions[0], "uploadId"),
            query(completions[1], "uploadId"),
            "the second completion is a new upload's"
        );
        assert!(
            completions
                .iter()
                .all(|request| conditioned(Provider::Aws, request))
        );
        assert_eq!(
            store.get(BUCKET, "lake/raced.bin").as_deref(),
            Some(&bytes[..])
        );
        assert_eq!(store.open_uploads(), 0);
    }

    /// A create's refusal is the conflict only where the store's own code
    /// says an object is at the key; any other `409` or `412` - a lease, a
    /// rehydration - is what the store said, status and code, and its key
    /// is no more taken than it was.
    #[test]
    fn a_refused_create_is_a_conflict_only_by_the_stores_own_code() {
        let store = store();
        let refusals = [
            (Provider::Aws, 412, "PreconditionFailed", true),
            (Provider::Google, 412, "conditionNotMet", true),
            (Provider::Azure, 409, "BlobAlreadyExists", true),
            (Provider::Azure, 412, "ConditionNotMet", true),
            (Provider::Azure, 412, "LeaseIdMissing", false),
            (Provider::Azure, 409, "BlobBeingRehydrated", false),
            (Provider::Google, 409, "conflict", false),
        ];
        for (provider, status, code, conflict) in refusals {
            let key = format!("lake/{}-{code}.json", provider.service());
            store.fail_next(status, code, 1);
            store.clear_requests();
            let error = file_on(&store, provider, &key)
                .create_bytes(b"{}")
                .expect_err("an injected refusal");
            assert_eq!(store.request_count(), 1, "{provider} {status} {code}");
            if conflict {
                assert!(
                    matches!(error, Error::Conflict { .. }),
                    "{provider} {status} {code}: {error}"
                );
                assert!(error.to_string().contains(&key), "{error}");
            } else {
                assert!(
                    matches!(
                        &error,
                        Error::Remote { status: answered, code: named, .. }
                            if *answered == status && named == code
                    ),
                    "{provider} {status} {code}: {error:?}"
                );
                assert!(!error.is_conflict(), "{error}");
            }
        }
    }
}

mod tail {
    //! [`IOBase::read_tail_bytes`], the footer-first read: one `GET` with a
    //! suffix range and no `HEAD` before it, the total learned from the
    //! answer's `Content-Range`.

    use crate::mod_::{BUCKET, file, file_on, payload, store};
    use crate::server::{FakeS3, Recorded};
    use yggdryl::s3::Provider;
    use yggdryl::{Error, IOBase};

    /// What each recorded request was: its method and the range it asked.
    fn asked(store: &FakeS3) -> Vec<(String, Option<String>)> {
        store
            .requests()
            .iter()
            .map(|request: &Recorded| {
                let range = request
                    .headers
                    .iter()
                    .find(|(name, _)| name == "range")
                    .map(|(_, value)| value.clone());
                (request.method.clone(), range)
            })
            .collect()
    }

    fn get(range: &str) -> (String, Option<String>) {
        ("GET".to_owned(), Some(range.to_owned()))
    }

    #[test]
    fn a_refused_tail_read_is_the_stores_refusal_and_never_an_empty_object() {
        let store = store();
        store.put(BUCKET, "lake/part.parquet", &payload(4096));
        let handle = file(&store, "lake/part.parquet");

        store.fail_next(403, "AccessDenied", 1);
        store.clear_requests();
        let error = handle.read_tail_bytes(8).expect_err("an injected refusal");
        assert!(
            matches!(&error, Error::Remote { status: 403, code, .. } if code == "AccessDenied"),
            "{error:?}"
        );
        assert_eq!(asked(&store), [get("bytes=-8")], "one request, refused");
        // A refusal teaches nothing: the size is asked of the store again.
        store.clear_requests();
        assert_eq!(handle.size(), 4096);
        assert_eq!(asked(&store), [("HEAD".to_owned(), None)]);
    }

    #[test]
    fn a_tail_read_is_one_suffix_ranged_get_and_states_the_size() {
        let store = store();
        let bytes = payload(4096);
        store.put(BUCKET, "lake/part.parquet", &bytes);
        let mut handle = file(&store, "lake/part.parquet");

        store.clear_requests();
        let (tail, total) = handle.read_tail_bytes(8).expect("the tail");
        assert_eq!(tail, &bytes[4088..]);
        assert_eq!(total, 4096);
        assert_eq!(
            asked(&store),
            [get("bytes=-8")],
            "one GET with a suffix range, no HEAD before it"
        );

        // The total the answer stated is kept as a listed size is: asking
        // for it costs nothing, and a range after it asks for no size.
        store.clear_requests();
        assert_eq!(handle.size(), 4096);
        assert_eq!(store.request_count(), 0, "the size the tail read stated");
        assert_eq!(
            handle.read_range_bytes(0, 4).expect("a window"),
            &bytes[..4]
        );
        assert_eq!(asked(&store), [get("bytes=0-3")]);

        // A write through the handle drops it, so the next size is asked.
        handle.write_all_bytes(b"PAR1").expect("a write");
        store.clear_requests();
        assert_eq!(handle.size(), 4);
        assert_eq!(asked(&store), [("HEAD".to_owned(), None)]);
    }

    #[test]
    fn a_window_wider_than_the_object_answers_all_of_it_for_the_same_get() {
        let store = store();
        let bytes = payload(100);
        store.put(BUCKET, "lake/part.parquet", &bytes);
        let handle = file(&store, "lake/part.parquet");

        store.clear_requests();
        assert_eq!(
            handle.read_tail_bytes(64 * 1024).expect("the whole object"),
            (bytes, 100)
        );
        assert_eq!(asked(&store), [get("bytes=-65536")]);
    }

    #[test]
    fn an_empty_object_and_a_missing_one_answer_nothing_for_one_get() {
        let store = store();
        store.put(BUCKET, "lake/empty.parquet", b"");
        let empty = file(&store, "lake/empty.parquet");

        // No suffix range is satisfiable over no bytes: the store's 416 is a
        // total of zero, and the zero is kept.
        store.clear_requests();
        assert_eq!(empty.read_tail_bytes(8).expect("nothing"), (Vec::new(), 0));
        assert_eq!(asked(&store), [get("bytes=-8")]);
        assert_eq!(store.requests()[0].status, 416);
        store.clear_requests();
        assert_eq!(empty.size(), 0);
        assert!(empty.exists(), "an empty object is there");
        assert_eq!(store.request_count(), 0);

        // Absence reads as emptiness, and teaches nothing to keep.
        let missing = file(&store, "lake/missing.parquet");
        store.clear_requests();
        assert_eq!(
            missing.read_tail_bytes(8).expect("nothing"),
            (Vec::new(), 0)
        );
        assert_eq!(asked(&store), [get("bytes=-8")]);
        assert_eq!(store.requests()[0].status, 404);
        store.clear_requests();
        assert!(!missing.exists());
        assert_eq!(asked(&store), [("HEAD".to_owned(), None)]);
    }

    #[test]
    fn a_store_that_reads_no_range_answers_its_tail_out_of_the_whole_object() {
        let store = store();
        let bytes = payload(4096);
        store.put(BUCKET, "lake/part.parquet", &bytes);
        store.ignore_ranges(true);
        let handle = file(&store, "lake/part.parquet");

        store.clear_requests();
        let (tail, total) = handle.read_tail_bytes(8).expect("the tail");
        assert_eq!(tail, &bytes[4088..]);
        assert_eq!(total, 4096, "the whole body's length is the total");
        assert_eq!(asked(&store), [get("bytes=-8")]);
        assert_eq!(store.requests()[0].status, 200);
    }

    #[test]
    fn google_reads_the_same_suffix_and_azure_counts_back_from_its_size() {
        let store = store();
        let bytes = payload(4096);
        for provider in [Provider::Aws, Provider::Google, Provider::Azure] {
            let key = format!("lake/{}.parquet", provider.service());
            store.put(BUCKET, &key, &bytes);
            let handle = file_on(&store, provider, &key);

            store.clear_requests();
            let (tail, total) = handle.read_tail_bytes(8).expect("the tail");
            assert_eq!(tail, &bytes[4088..], "{provider}");
            assert_eq!(total, 4096, "{provider}");
            let expected = match provider {
                // Azure Blob Storage reads no suffix range: its size, then
                // the range counted back from it.
                Provider::Azure => vec![("HEAD".to_owned(), None), get("bytes=4088-4095")],
                Provider::Aws | Provider::Google => vec![get("bytes=-8")],
            };
            assert_eq!(asked(&store), expected, "{provider}");
            if provider == Provider::Google {
                let media = store.requests()[0]
                    .query
                    .iter()
                    .any(|(name, value)| name == "alt" && value == "media");
                assert!(media, "the media download carries the range");
            }
        }
    }

    #[test]
    fn an_open_handle_counts_back_from_the_size_it_holds_and_a_staged_one_asks_nothing() {
        let store = store();
        let bytes = payload(4096);
        store.put(BUCKET, "lake/part.parquet", &bytes);
        let mut handle = file(&store, "lake/part.parquet");
        handle.open().expect("an open");

        store.clear_requests();
        let (tail, total) = handle.read_tail_bytes(8).expect("the tail");
        assert_eq!((tail.as_slice(), total), (&bytes[4088..], 4096));
        assert_eq!(asked(&store), [get("bytes=4088-4095")]);
        handle.close().expect("a close");

        // An open scope that found nothing asks nothing more.
        let mut missing = file(&store, "lake/missing.parquet");
        missing.open().expect("an open of nothing");
        store.clear_requests();
        assert_eq!(
            missing.read_tail_bytes(8).expect("nothing"),
            (Vec::new(), 0)
        );
        assert_eq!(store.request_count(), 0);
        missing.close().expect("a close");

        // A staged write is what the handle answers, from memory.
        let mut staged = file(&store, "lake/staged.parquet");
        staged.truncate(0).expect("an empty stage");
        staged.pwrite(0, b"PAR1....PAR1").expect("a staged write");
        store.clear_requests();
        assert_eq!(
            staged.read_tail_bytes(4).expect("the staged tail"),
            (b"PAR1".to_vec(), 12)
        );
        assert_eq!(store.request_count(), 0);
        staged.close().expect("the stage published");
    }
}

/// What a Parquet read over a closed object costs: its end, the footer in
/// it, and the length learned beside it, in one suffix-ranged `GET` - no
/// `HEAD` for the size the footer is counted back from.
#[cfg(feature = "parquet")]
mod parquet {
    use std::sync::Arc;

    use arrow_array::{Int64Array, RecordBatch};
    use arrow_schema::{DataType, Field, Schema};
    use yggdryl::IOMedia;

    use crate::mod_::{file, store};
    use crate::server::FakeS3;

    /// `count` rows of one column whose values do not compress, so the file
    /// is about eight bytes a row.
    fn rows(count: i64) -> RecordBatch {
        let schema = Arc::new(Schema::new(vec![Field::new("id", DataType::Int64, false)]));
        // The golden-ratio multiplier, `0x9E37_79B9_7F4A_7C15` as an `i64`.
        let values = (0..count).map(|row| row.wrapping_mul(-7_046_029_254_386_353_131));
        RecordBatch::try_new(schema, vec![Arc::new(Int64Array::from_iter_values(values))])
            .expect("a batch")
    }

    /// Write `batch` as the object `key` through its own handle.
    fn written(store: &FakeS3, key: &str, batch: RecordBatch) {
        let mut handle = file(store, key);
        let options = handle.record_options().expect("an encoding");
        handle
            .overwrite_arrow_batch(batch, &options)
            .expect("a written batch");
    }

    /// Each recorded request as its method and the range it asked.
    fn asked(store: &FakeS3) -> Vec<String> {
        store
            .requests()
            .iter()
            .map(|request| {
                let range = request
                    .headers
                    .iter()
                    .find(|(name, _)| name == "range")
                    .map_or("whole", |(_, value)| value.as_str());
                format!("{} {range}", request.method)
            })
            .collect()
    }

    #[test]
    fn a_footer_read_is_one_suffix_ranged_get_and_no_head() {
        let store = store();
        written(&store, "lake/part.parquet", rows(64));
        let handle = file(&store, "lake/part.parquet");
        let options = handle.record_options().expect("an encoding");

        // The schema and the row count are the footer's: the file's last
        // 64 KiB, which a small file is all of.
        store.clear_requests();
        let field = handle.read_arrow_field(&options).expect("the schema");
        assert!(field.to_string().contains("id"), "{field}");
        assert_eq!(asked(&store), ["GET bytes=-65536"]);
        let handle = file(&store, "lake/part.parquet");
        store.clear_requests();
        assert_eq!(handle.row_size().expect("a row count"), 64);
        assert_eq!(asked(&store), ["GET bytes=-65536"]);

        // A read of every row takes the file's last MiB, which a file of at
        // most that much arrives whole in.
        let handle = file(&store, "lake/part.parquet");
        store.clear_requests();
        let read: usize = handle
            .read_arrow_reader(&options)
            .expect("a reader")
            .map(|batch| batch.expect("a batch").num_rows())
            .sum();
        assert_eq!(read, 64);
        assert_eq!(asked(&store), ["GET bytes=-1048576"]);
    }

    #[test]
    fn a_read_of_a_file_past_a_mebibyte_takes_its_end_then_the_chunks_it_lacks() {
        let store = store();
        written(&store, "lake/large.parquet", rows(200_000));
        let size = store
            .get(crate::mod_::BUCKET, "lake/large.parquet")
            .expect("the object")
            .len();
        assert!(size > 1024 * 1024, "{size} bytes");
        let handle = file(&store, "lake/large.parquet");
        let options = handle.record_options().expect("an encoding");

        store.clear_requests();
        let read: usize = handle
            .read_arrow_reader(&options)
            .expect("a reader")
            .map(|batch| batch.expect("a batch").num_rows())
            .sum();
        assert_eq!(read, 200_000);
        let asked = asked(&store);
        assert_eq!(asked[0], "GET bytes=-1048576", "{asked:?}");
        assert!(
            asked[1..]
                .iter()
                .all(|request| request.starts_with("GET bytes=")
                    && !request.starts_with("GET bytes=-")),
            "the chunks are ranged reads, and nothing asks the size: {asked:?}"
        );
        assert_eq!(
            asked.len(),
            2,
            "the end, then the part of the one column chunk it lacks: {asked:?}"
        );
        // The part it lacks runs from the chunk's start, right after the
        // leading magic, to the byte before the end in hand: nothing the end
        // holds is asked for twice.
        let held_from = size - 1024 * 1024;
        assert_eq!(
            asked[1],
            format!("GET bytes=4-{}", held_from - 1),
            "{asked:?}"
        );
    }
}
