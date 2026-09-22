//! `rust/src/text/reader.rs`: one shared window, cut into physical lines.
//!
//! The window is an allocation strategy rather than a contract - a caller
//! reads lines and never the page they are ranges of - so the splitter and
//! its window size are reached through `yggdryl::internals`.

#[cfg(feature = "internals")]
mod internal {
    use std::io::Read;

    use yggdryl::internals::text_reader::{Lines, WINDOW_SIZE};
    use yggdryl::text::LineSep;

    struct Chunked {
        bytes: std::io::Cursor<Vec<u8>>,
        size: usize,
    }

    impl Read for Chunked {
        fn read(&mut self, target: &mut [u8]) -> std::io::Result<usize> {
            let size = target.len().min(self.size);
            self.bytes.read(&mut target[..size])
        }
    }

    fn read(input: &[u8], linesep: Option<&LineSep>) -> Vec<Vec<u8>> {
        let mut lines = Lines::new(std::io::Cursor::new(input));
        let mut values = Vec::new();
        while let Some(line) = lines.next_line(linesep) {
            values.push(line.unwrap());
        }
        values
    }

    #[test]
    fn flexible_and_pinned_terminators_stream_lines() {
        assert_eq!(
            read(b"a\r\nb\nc\rd", None),
            [b"a".to_vec(), b"b".to_vec(), b"c".to_vec(), b"d".to_vec()]
        );
        assert_eq!(
            read(b"a\nb\r\nc", Some(&LineSep::CRLF)),
            [b"a\nb".to_vec(), b"c".to_vec()]
        );
        assert_eq!(read(b"\xef\xbb\xbfa\n", None), [b"\xef\xbb\xbfa".to_vec()]);
    }

    #[test]
    fn terminators_can_cross_short_source_reads() {
        let mut flexible = Lines::new(Chunked {
            bytes: std::io::Cursor::new(b"a\r\nb\rc\nlast".to_vec()),
            size: 1,
        });
        let mut values = Vec::new();
        while let Some(line) = flexible.next_line(None) {
            values.push(line.unwrap());
        }
        assert_eq!(
            values,
            [
                b"a".to_vec(),
                b"b".to_vec(),
                b"c".to_vec(),
                b"last".to_vec()
            ]
        );

        let mut pinned = Lines::new(Chunked {
            bytes: std::io::Cursor::new(b"a\r\nb\r\nc".to_vec()),
            size: 1,
        });
        let mut values = Vec::new();
        while let Some(line) = pinned.next_line(Some(&LineSep::CRLF)) {
            values.push(line.unwrap());
        }
        assert_eq!(values, [b"a".to_vec(), b"b".to_vec(), b"c".to_vec()]);
    }

    #[test]
    fn a_line_larger_than_the_window_is_returned_as_bounded_parts() {
        let mut input = vec![b'x'; WINDOW_SIZE * 3 + 7];
        input.extend_from_slice(b"\nnext");
        let mut lines = Lines::new(std::io::Cursor::new(input));
        let mut sizes = Vec::new();
        loop {
            let part = lines.next_part(None).unwrap().unwrap();
            sizes.push(part.len());
            if part.end {
                break;
            }
        }
        assert!(sizes.len() >= 3);
        assert!(sizes.iter().all(|size| *size <= WINDOW_SIZE));
        assert_eq!(lines.next_line(None).unwrap().unwrap(), b"next");
    }
}

mod text {
    use arrow_array::{Array as _, StringArray};
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

    #[test]
    fn framing_carries_a_record_across_input_windows_and_output_batches() {
        let mut bytes = b"[A] first\n".to_vec();
        bytes.extend(std::iter::repeat_n(
            b'x',
            yggdryl::DEFAULT_STREAM_BATCH_SIZE + 17,
        ));
        bytes.extend_from_slice(b"\nlast continuation\n[B] next\nend\n");
        let source = named("windows.log", &bytes);
        let mut options = framed(r"^\[(?<kind>[A-Z])\] ");
        options.set_batch_row_size(Some(1));

        let batches = collect(&source, options);
        assert_eq!(batches.len(), 2);
        let bodies = bodies(&batches);
        assert_eq!(
            bodies[0].len(),
            4 + 6 + yggdryl::DEFAULT_STREAM_BATCH_SIZE + 17 + 18
        );
        assert!(bodies[0].starts_with(b"[A] first\nxxxxxxxx"));
        assert!(bodies[0].ends_with(b"\nlast continuation"));
        assert_eq!(bodies[1], b"[B] next\nend");
    }

    /// What one text read costs the store underneath it.
    ///
    /// A record read decodes through one sequential open, so the question a remote
    /// store cares about is how many times that open is asked for bytes. The
    /// decoder pulls in its own small increments - gzip reads 32 KiB at a time -
    /// so without a fetch window between them a gigabyte-scale object would cost
    /// tens of thousands of round trips for bytes it is going to read in order
    /// anyway.
    mod fetching {

        use std::any::Any;
        use std::sync::{Arc, Mutex};
        use yggdryl::text::{Text, TextOptions};

        use yggdryl::fs::{
            BoundLocation, ByteReader, ByteWriter, FileInfo, FileInfos, FileSelector, FileSystem,
            MemoryFileSystem, OutputMetadata, RandomAccessReader,
        };
        use yggdryl::{Codec, DEFAULT_FETCH_BYTE_SIZE, IOBase as _, IOMedia as _, Url};

        const ROWHEADER: &str = r"^\[(?<level>[A-Z]+)\] ";

        /// One filesystem that records the size of every read its streams serve.
        struct Counting {
            inner: MemoryFileSystem,
            reads: Arc<Mutex<Vec<usize>>>,
            opens: Arc<Mutex<usize>>,
        }

        struct CountingReader {
            inner: Box<dyn ByteReader>,
            reads: Arc<Mutex<Vec<usize>>>,
        }

        impl ByteReader for CountingReader {
            fn read(&mut self, buffer: &mut [u8]) -> yggdryl::Result<usize> {
                self.reads.lock().unwrap().push(buffer.len());
                self.inner.read(buffer)
            }

            fn tell(&self) -> u64 {
                self.inner.tell()
            }

            fn close(&mut self) -> yggdryl::Result<()> {
                self.inner.close()
            }

            fn closed(&self) -> bool {
                self.inner.closed()
            }

            fn as_any(&self) -> &dyn Any {
                self
            }

            fn into_any(self: Box<Self>) -> Box<dyn Any> {
                self
            }
        }

        impl FileSystem for Counting {
            fn type_name(&self) -> &str {
                "counting"
            }

            fn equals(&self, other: &dyn FileSystem) -> bool {
                other.as_any().downcast_ref::<Self>().is_some()
            }

            fn normalize_path(&self, path: &str) -> yggdryl::Result<String> {
                self.inner.normalize_path(path)
            }

            fn file_info(&self, path: &str) -> yggdryl::Result<FileInfo> {
                self.inner.file_info(path)
            }

            fn list(&self, selector: &FileSelector) -> FileInfos {
                self.inner.list(selector)
            }

            fn create_dir(&self, path: &str, recursive: bool) -> yggdryl::Result<()> {
                self.inner.create_dir(path, recursive)
            }

            fn delete_dir(&self, path: &str) -> yggdryl::Result<()> {
                self.inner.delete_dir(path)
            }

            fn delete_dir_contents(&self, path: &str, missing_dir_ok: bool) -> yggdryl::Result<()> {
                self.inner.delete_dir_contents(path, missing_dir_ok)
            }

            fn delete_root_dir_contents(&self) -> yggdryl::Result<()> {
                self.inner.delete_root_dir_contents()
            }

            fn delete_file(&self, path: &str) -> yggdryl::Result<()> {
                self.inner.delete_file(path)
            }

            fn copy_file(&self, source: &str, target: &str) -> yggdryl::Result<()> {
                self.inner.copy_file(source, target)
            }

            fn move_file(&self, source: &str, target: &str) -> yggdryl::Result<()> {
                self.inner.move_file(source, target)
            }

            fn open_input_file(&self, path: &str) -> yggdryl::Result<Box<dyn RandomAccessReader>> {
                self.inner.open_input_file(path)
            }

            fn open_input_stream(&self, path: &str) -> yggdryl::Result<Box<dyn ByteReader>> {
                *self.opens.lock().unwrap() += 1;
                Ok(Box::new(CountingReader {
                    inner: self.inner.open_input_stream(path)?,
                    reads: Arc::clone(&self.reads),
                }))
            }

            fn open_output_stream(
                &self,
                path: &str,
                metadata: Option<&OutputMetadata>,
            ) -> yggdryl::Result<Box<dyn ByteWriter>> {
                self.inner.open_output_stream(path, metadata)
            }

            fn open_append_stream(
                &self,
                path: &str,
                metadata: Option<&OutputMetadata>,
            ) -> yggdryl::Result<Box<dyn ByteWriter>> {
                self.inner.open_append_stream(path, metadata)
            }

            fn as_any(&self) -> &dyn Any {
                self
            }
        }

        /// Text that gzip cannot shrink away, so the encoded object spans several
        /// fetch windows and records straddle every boundary between them.
        fn payload(lines: usize) -> (Vec<u8>, Vec<Vec<u8>>) {
            const ALPHABET: &[u8] =
                b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
            let mut state = 0x2545_F491_u32;
            let mut plain = Vec::new();
            let mut bodies = Vec::new();
            for index in 0..lines {
                let mut body = format!("id={index} ").into_bytes();
                for _ in 0..1_024 {
                    state ^= state << 13;
                    state ^= state >> 17;
                    state ^= state << 5;
                    body.push(ALPHABET[state as usize % ALPHABET.len()]);
                }
                plain.extend_from_slice(b"[INFO] ");
                plain.extend_from_slice(&body);
                plain.push(b'\n');
                bodies.push([b"[INFO] ".as_slice(), &body].concat());
            }
            (plain, bodies)
        }

        fn framed() -> TextOptions {
            TextOptions::new()
                .try_with_rowheader(ROWHEADER)
                .unwrap()
                .with_framing(true)
        }

        fn located(name: &str, bytes: &[u8]) -> (yggdryl::fs::FsFile, Arc<Counting>) {
            let filesystem = Arc::new(Counting {
                inner: MemoryFileSystem::new(),
                reads: Arc::new(Mutex::new(Vec::new())),
                opens: Arc::new(Mutex::new(0)),
            });
            filesystem
                .inner
                .open_output_stream(name, None)
                .unwrap()
                .write(bytes)
                .unwrap();
            let bound =
                BoundLocation::new(Arc::clone(&filesystem) as Arc<dyn FileSystem>, name, None)
                    .unwrap();
            let mut handle = yggdryl::fs::FsFile::new(bound);
            handle.set_media_type(
                Url::from_str(&format!("file:///{name}"))
                    .unwrap()
                    .media_type(),
            );
            (handle, filesystem)
        }

        #[test]
        fn a_compressed_leaf_streams_one_open_in_whole_fetch_windows() {
            let (plain, expected) = payload(3_000);
            let encoded = Codec::Gzip.dump(&plain).unwrap();
            assert!(
                encoded.len() > 2 * DEFAULT_FETCH_BYTE_SIZE,
                "the object must span several windows, got {} bytes",
                encoded.len()
            );
            let (handle, filesystem) = located("app.log.gz", &encoded);

            let read = super::collect(&handle, framed());
            assert_eq!(super::bodies(&read), expected);

            // One open, and every ask but the last is a whole window: the decoder's
            // own 32 KiB appetite never reaches the store.
            assert_eq!(*filesystem.opens.lock().unwrap(), 1);
            let reads = filesystem.reads.lock().unwrap().clone();
            assert!(
                reads.iter().all(|size| *size == DEFAULT_FETCH_BYTE_SIZE),
                "every fetch asks for a whole window, got {reads:?}"
            );
            let windows = encoded.len().div_ceil(DEFAULT_FETCH_BYTE_SIZE);
            assert!(
                (windows..=windows + 1).contains(&reads.len()),
                "one fetch per window of {} encoded bytes, at most one more to see              the end, got {} fetches",
                encoded.len(),
                reads.len()
            );

            // Counting the rows, rather than materializing them, reads the same
            // way: one open, whole windows.
            filesystem.reads.lock().unwrap().clear();
            *filesystem.opens.lock().unwrap() = 0;
            assert_eq!(
                Text::new(handle).with_options(framed()).row_size().unwrap(),
                3_000
            );
            assert_eq!(*filesystem.opens.lock().unwrap(), 1);
            // Counting asks for a window at a time as well; the shorter final ask
            // is the tail of the last window, not a small request of its own.
            let counted = filesystem.reads.lock().unwrap().clone();
            assert_eq!(counted.first(), Some(&DEFAULT_FETCH_BYTE_SIZE));
            assert!(
                counted.iter().all(|size| *size <= DEFAULT_FETCH_BYTE_SIZE),
                "counting rows never asks beyond one window, got {counted:?}"
            );
            assert!(
                counted.len() <= windows + 1,
                "counting rows fetches one window at a time, got {counted:?}"
            );
        }

        #[test]
        fn an_empty_compressed_leaf_is_not_probed_a_byte_at_a_time() {
            let (handle, filesystem) = located("empty.log.gz", &[]);

            assert_eq!(
                handle.read_arrow_reader(&framed().into()).unwrap().count(),
                0
            );
            assert_eq!(*filesystem.opens.lock().unwrap(), 1);
            assert_eq!(
                filesystem.reads.lock().unwrap().as_slice(),
                [DEFAULT_FETCH_BYTE_SIZE],
                "emptiness is answered by the first window, not by a one-byte read"
            );
        }
    }
}
