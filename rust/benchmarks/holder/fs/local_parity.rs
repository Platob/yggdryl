//! Deterministic local-filesystem parity gate.
//!
//! The ordinary Criterion groups describe performance. This focused gate
//! compares the same 64 MiB operation, at the same 64 KiB chunk size, through
//! the public filesystem handle and directly through `std::fs`. One warm-up is
//! discarded and five samples produce the reported median.

use std::any::Any;
use std::fs::OpenOptions;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use criterion::Criterion;
use yggdryl::IOBase;
use yggdryl::Result;
use yggdryl::holder::fs::{
    ByteReader, ByteWriter, File as FsFile, FileInfo, FileInfos, FileSelector, FileSystem,
    LocalFileSystem, OutputMetadata, RandomAccessReader,
};

const PAYLOAD_BYTES: u64 = 64 * 1024 * 1024;
const CHUNK_BYTES: usize = 64 * 1024;
const SAMPLES: usize = 5;

pub(crate) fn local_parity_benchmarks(_: &mut Criterion) {
    let scratch = Scratch::new();
    let source_path = scratch.path("source.bin");
    let direct_write_path = scratch.path("direct-write.bin");
    let wrapped_write_path = scratch.path("wrapped-write.bin");
    let direct_copy_path = scratch.path("direct-copy.bin");
    let wrapped_copy_path = scratch.path("wrapped-copy.bin");
    let direct_copy_temporary = temporary_path(&direct_copy_path);
    let chunk = fixture_chunk();

    write_direct(&source_path, &chunk);
    verify_file(&source_path, &chunk);

    let local: Arc<dyn FileSystem> = Arc::new(LocalFileSystem::new());
    let distinct_local: Arc<dyn FileSystem> = Arc::new(DistinctLocalFileSystem::default());
    let source = FsFile::from_path(local.clone(), raw_path(&source_path), None)
        .expect("the local source must bind");
    let wrapped_write = FsFile::from_path(local, raw_path(&wrapped_write_path), None)
        .expect("the local write target must bind");
    let mut wrapped_copy = FsFile::from_path(distinct_local, raw_path(&wrapped_copy_path), None)
        .expect("the cross-domain local target must bind");

    let read = measure_pair(
        "read",
        || {},
        || read_direct(&source_path),
        || {},
        || read_wrapped(&source),
    );
    let write = measure_pair(
        "write",
        || remove_file_if_present(&direct_write_path),
        || write_direct(&direct_write_path, &chunk),
        || remove_file_if_present(&wrapped_write_path),
        || write_wrapped(&wrapped_write, &chunk),
    );
    let copy = measure_pair(
        "copy",
        || {
            remove_file_if_present(&direct_copy_path);
            remove_file_if_present(&direct_copy_temporary);
        },
        || copy_direct(&source_path, &direct_copy_path),
        || remove_file_if_present(&wrapped_copy_path),
        || {
            source
                .copy_into(&mut wrapped_copy)
                .expect("the wrapped copy must complete")
        },
    );

    verify_file(&direct_write_path, &chunk);
    verify_file(&wrapped_write_path, &chunk);
    verify_file(&direct_copy_path, &chunk);
    verify_file(&wrapped_copy_path, &chunk);

    println!(
        "local filesystem parity: median throughput after warm-up, {} MiB, {} KiB chunks",
        PAYLOAD_BYTES / (1024 * 1024),
        CHUNK_BYTES / 1024
    );
    for metric in [read, write, copy] {
        let direct = throughput(metric.direct);
        let wrapped = throughput(metric.wrapped);
        let ratio = wrapped / direct;
        println!(
            "  {:>5}: direct {direct:>9.1} MiB/s, yggdryl {wrapped:>9.1} MiB/s, ratio {ratio:.3}",
            metric.name
        );
        assert!(
            ratio >= 0.75,
            "{}: yggdryl throughput ({wrapped:.1} MiB/s) is more than 25% slower than direct local ({direct:.1} MiB/s)",
            metric.name
        );
    }
}

struct Metric {
    name: &'static str,
    direct: Duration,
    wrapped: Duration,
}

fn measure_pair(
    name: &'static str,
    mut prepare_direct: impl FnMut(),
    mut direct: impl FnMut() -> u64,
    mut prepare_wrapped: impl FnMut(),
    mut wrapped: impl FnMut() -> u64,
) -> Metric {
    let _ = elapsed(&mut prepare_direct, &mut direct);
    let _ = elapsed(&mut prepare_wrapped, &mut wrapped);

    let mut direct_samples = Vec::with_capacity(SAMPLES);
    let mut wrapped_samples = Vec::with_capacity(SAMPLES);
    for sample in 0..SAMPLES {
        if sample % 2 == 0 {
            direct_samples.push(elapsed(&mut prepare_direct, &mut direct));
            wrapped_samples.push(elapsed(&mut prepare_wrapped, &mut wrapped));
        } else {
            wrapped_samples.push(elapsed(&mut prepare_wrapped, &mut wrapped));
            direct_samples.push(elapsed(&mut prepare_direct, &mut direct));
        }
    }
    direct_samples.sort_unstable();
    wrapped_samples.sort_unstable();
    Metric {
        name,
        direct: direct_samples[SAMPLES / 2],
        wrapped: wrapped_samples[SAMPLES / 2],
    }
}

fn elapsed(prepare: &mut impl FnMut(), operation: &mut impl FnMut() -> u64) -> Duration {
    prepare();
    let started = Instant::now();
    let transferred = operation();
    let elapsed = started.elapsed();
    assert_eq!(transferred, PAYLOAD_BYTES, "the complete payload must move");
    elapsed
}

fn throughput(duration: Duration) -> f64 {
    PAYLOAD_BYTES as f64 / (1024.0 * 1024.0) / duration.as_secs_f64()
}

fn fixture_chunk() -> Vec<u8> {
    (0..CHUNK_BYTES).map(|index| (index % 251) as u8).collect()
}

fn raw_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn read_direct(path: &Path) -> u64 {
    let mut file = std::fs::File::open(path).expect("the direct input must open");
    read_std(&mut file)
}

fn read_std(reader: &mut impl Read) -> u64 {
    let mut buffer = vec![0_u8; CHUNK_BYTES];
    let mut total = 0_u64;
    loop {
        let read = reader
            .read(&mut buffer)
            .expect("the direct input must read");
        if read == 0 {
            return total;
        }
        total += read as u64;
    }
}

fn read_wrapped(file: &FsFile) -> u64 {
    let mut reader = file
        .open_input_stream()
        .expect("the wrapped input must open");
    let mut buffer = vec![0_u8; CHUNK_BYTES];
    let mut total = 0_u64;
    loop {
        let read = reader
            .read(&mut buffer)
            .expect("the wrapped input must read");
        if read == 0 {
            break;
        }
        total += read as u64;
    }
    reader.close().expect("the wrapped input must close");
    total
}

fn write_direct(path: &Path, chunk: &[u8]) -> u64 {
    let mut writer = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(path)
        .expect("the direct output must open");
    let written = write_std(&mut writer, chunk);
    writer.flush().expect("the direct output must flush");
    written
}

fn write_std(writer: &mut impl Write, chunk: &[u8]) -> u64 {
    let mut total = 0_u64;
    while total < PAYLOAD_BYTES {
        let remaining = usize::try_from(PAYLOAD_BYTES - total).unwrap_or(CHUNK_BYTES);
        let length = remaining.min(CHUNK_BYTES);
        writer
            .write_all(&chunk[..length])
            .expect("the direct output must write");
        total += length as u64;
    }
    total
}

fn write_wrapped(file: &FsFile, chunk: &[u8]) -> u64 {
    let mut writer = file
        .open_output_stream(None)
        .expect("the wrapped output must open");
    let mut total = 0_u64;
    while total < PAYLOAD_BYTES {
        let remaining = usize::try_from(PAYLOAD_BYTES - total).unwrap_or(CHUNK_BYTES);
        let length = remaining.min(CHUNK_BYTES);
        let mut position = 0;
        while position < length {
            let written = writer
                .write(&chunk[position..length])
                .expect("the wrapped output must write");
            assert_ne!(written, 0, "the wrapped output must make progress");
            position += written;
        }
        total += length as u64;
    }
    writer.close().expect("the wrapped output must close");
    total
}

fn copy_direct(source: &Path, target: &Path) -> u64 {
    let temporary = temporary_path(target);
    let mut reader = std::fs::File::open(source).expect("the direct copy input must open");
    let mut writer = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(&temporary)
        .expect("the direct copy output must open");
    let mut buffer = vec![0_u8; CHUNK_BYTES];
    let mut total = 0_u64;
    loop {
        let read = reader
            .read(&mut buffer)
            .expect("the direct copy input must read");
        if read == 0 {
            break;
        }
        writer
            .write_all(&buffer[..read])
            .expect("the direct copy output must write");
        total += read as u64;
    }
    writer.flush().expect("the direct copy output must flush");
    drop(writer);
    std::fs::rename(temporary, target).expect("the direct copy must publish");
    total
}

fn temporary_path(target: &Path) -> PathBuf {
    target.with_extension("direct-transfer")
}

fn remove_file_if_present(path: &Path) {
    match std::fs::remove_file(path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => panic!("failed to remove benchmark file {path:?}: {error}"),
    }
}

fn verify_file(path: &Path, expected_chunk: &[u8]) {
    assert_eq!(
        std::fs::metadata(path)
            .expect("the benchmark output must exist")
            .len(),
        PAYLOAD_BYTES,
        "the benchmark output must have the complete size"
    );
    let mut file = std::fs::File::open(path).expect("the benchmark output must open");
    let mut actual = vec![0_u8; CHUNK_BYTES];
    file.read_exact(&mut actual)
        .expect("the benchmark output must contain one chunk");
    assert_eq!(actual, expected_chunk, "the benchmark output must match");
}

struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        let mut root = yggdryl::holder::local::Folder::temporary()
            .expect("the temporary directory")
            .path()
            .expect("a platform path");
        root.push(format!("yggdryl-fs-parity-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("a writable temporary root");
        Self(root)
    }

    fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A second equality domain over local files forces the public bounded-copy
/// path while retaining an identical operating-system backend.
#[derive(Clone, Copy, Debug, Default)]
struct DistinctLocalFileSystem(LocalFileSystem);

impl FileSystem for DistinctLocalFileSystem {
    fn type_name(&self) -> &str {
        self.0.type_name()
    }

    fn equals(&self, other: &dyn FileSystem) -> bool {
        other.as_any().is::<Self>()
    }

    fn normalize_path(&self, path: &str) -> Result<String> {
        self.0.normalize_path(path)
    }

    fn file_info(&self, path: &str) -> Result<FileInfo> {
        self.0.file_info(path)
    }

    fn list(&self, selector: &FileSelector) -> FileInfos {
        self.0.list(selector)
    }

    fn create_dir(&self, path: &str, recursive: bool) -> Result<()> {
        self.0.create_dir(path, recursive)
    }

    fn delete_dir(&self, path: &str) -> Result<()> {
        self.0.delete_dir(path)
    }

    fn delete_dir_contents(&self, path: &str, missing_dir_ok: bool) -> Result<()> {
        self.0.delete_dir_contents(path, missing_dir_ok)
    }

    fn delete_root_dir_contents(&self) -> Result<()> {
        self.0.delete_root_dir_contents()
    }

    fn delete_file(&self, path: &str) -> Result<()> {
        self.0.delete_file(path)
    }

    fn copy_file(&self, source: &str, target: &str) -> Result<()> {
        self.0.copy_file(source, target)
    }

    fn move_file(&self, source: &str, target: &str) -> Result<()> {
        self.0.move_file(source, target)
    }

    fn open_input_file(&self, path: &str) -> Result<Box<dyn RandomAccessReader>> {
        self.0.open_input_file(path)
    }

    fn open_input_stream(&self, path: &str) -> Result<Box<dyn ByteReader>> {
        self.0.open_input_stream(path)
    }

    fn open_output_stream(
        &self,
        path: &str,
        metadata: Option<&OutputMetadata>,
    ) -> Result<Box<dyn ByteWriter>> {
        self.0.open_output_stream(path, metadata)
    }

    fn open_append_stream(
        &self,
        path: &str,
        metadata: Option<&OutputMetadata>,
    ) -> Result<Box<dyn ByteWriter>> {
        self.0.open_append_stream(path, metadata)
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}
