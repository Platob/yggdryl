//! `rust/src/iobase/hierarchy.rs`: owned readers retain one exact byte source.

#![cfg(feature = "internals")]

use std::cell::Cell;
use std::sync::Arc;

use yggdryl::holder::{Buffer, Holder};
use yggdryl::internals::bytestream::{ByteSource, from_source};
use yggdryl::internals::iobase_hierarchy::owned_handle;
use yggdryl::{ByteStream, Error, IOBase, IOMedia, MediaType, Result};

struct Sequential {
    bytes: Buffer,
    fail_at: Option<usize>,
    streams: Cell<usize>,
}

impl IOMedia for Sequential {
    yggdryl::impl_default_iomedia!();
}

impl IOBase for Sequential {
    yggdryl::delegate_iobase!(bytes: pwrite, capacity, reserve, truncate, uri, url,
        media_type, set_media_type);

    fn size(&self) -> u64 {
        panic!("owned intake must not stat a sequential source")
    }

    fn pread(&self, _: u64, _: &mut [u8]) -> Result<usize> {
        panic!("owned intake must retain the source's native byte stream")
    }

    fn pstream_bytes(&self, position: u64, batch_size: usize) -> Result<ByteStream<'_>> {
        assert_eq!(position, 0);
        self.streams.set(self.streams.get() + 1);
        from_source(
            Pieces {
                source: self,
                position: 0,
            },
            batch_size,
        )
    }
}

struct Pieces<'a> {
    source: &'a Sequential,
    position: usize,
}

impl ByteSource for Pieces<'_> {
    fn read_bytes(&mut self, target: &mut [u8]) -> Result<usize> {
        if self.source.fail_at == Some(self.position) {
            return Err(Error::InvalidRecord {
                path: "owned-source".into(),
                reason: "late failure".into(),
            });
        }
        let bytes = self.source.bytes.as_slice();
        let end = self.source.fail_at.unwrap_or(bytes.len()).min(bytes.len());
        let count = (end - self.position).min(target.len()).min(97);
        target[..count].copy_from_slice(&bytes[self.position..self.position + count]);
        self.position += count;
        Ok(count)
    }
}

fn declared() -> MediaType {
    MediaType::from_str(
        "text/plain;charset=windows-1252;encodings=application/gzip,application/zstd",
    )
    .unwrap()
}

#[test]
fn owned_handle_materializes_one_native_stream_and_keeps_all_media_facts() {
    for length in [0, 64, 4096, 2 * 64 * 1024 + 11] {
        let expected = (0..length).map(|at| (at % 251) as u8).collect::<Vec<_>>();
        let media = declared();
        let owned = {
            let source = Sequential {
                bytes: Buffer::from_bytes(expected.clone()).with_media_type(media.clone()),
                fail_at: None,
                streams: Cell::new(0),
            };
            let owned = owned_handle(&source).unwrap();
            assert_eq!(source.streams.get(), 1);
            assert_eq!(source.bytes.as_slice(), expected);
            assert_eq!(source.media_type(), &media);
            owned
        };
        assert_eq!(owned.media_type(), &media);
        let Holder::Buffer(buffer) = owned else {
            panic!("unlocated bytes need an owned buffer")
        };
        assert_eq!(buffer.as_slice(), expected);
    }
}

#[test]
fn owned_handle_late_failure_keeps_its_typed_location_and_public_copy_atomicity() {
    for fail_at in [3, 64 * 1024 + 3] {
        let source = Sequential {
            bytes: Buffer::from_bytes(vec![7; fail_at + 1]).with_media_type(declared()),
            fail_at: Some(fail_at),
            streams: Cell::new(0),
        };
        let refused = owned_handle(&source).unwrap_err();
        assert!(matches!(refused, Error::InvalidRecord { path, reason }
            if path == "owned-source" && reason == "late failure"));
        assert_eq!(source.streams.get(), 1);

        let old_media = MediaType::from_str("application/json;charset=utf-8").unwrap();
        let mut existing =
            Buffer::from_bytes(b"retained".to_vec()).with_media_type(old_media.clone());
        let refused = source.copy_into(&mut existing).unwrap_err();
        assert!(matches!(refused, Error::InvalidRecord { path, reason }
            if path == "owned-source" && reason == "late failure"));
        assert_eq!(existing.as_slice(), b"retained");
        assert_eq!(existing.media_type(), &old_media);
        assert_eq!(source.streams.get(), 2);
    }
}

#[test]
fn owned_handle_reopens_a_bound_resource_without_materializing_it() {
    let filesystem: Arc<dyn yggdryl::fs::FileSystem> =
        Arc::new(yggdryl::fs::MemoryFileSystem::new());
    // The resource is deliberately absent: opening the owned view remains lazy.
    let mut source =
        yggdryl::fs::FsFile::from_path(Arc::clone(&filesystem), "absent", None).unwrap();
    source.set_media_type(declared());
    let owned = owned_handle(&source).unwrap();
    assert_eq!(owned.media_type(), source.media_type());
    let Holder::FsFile(file) = owned else {
        panic!("a bound resource stays bound")
    };
    assert!(Arc::ptr_eq(file.filesystem(), &filesystem));
    assert_eq!(file.path(), "absent");
}
