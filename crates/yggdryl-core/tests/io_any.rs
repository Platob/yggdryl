//! Functional tests for [`open`](yggdryl_core::io::open) / [`AnyIO`](yggdryl_core::io::AnyIO) —
//! the scheme-dispatching `open()` entry point and the uniform handle it returns.

use yggdryl_core::io::memory::IOBase;
use yggdryl_core::io::{open, open_str, AnyIO};
use yggdryl_core::uri::Uri;

#[test]
fn open_mem_scheme_yields_a_heap_backed_handle() {
    let mut io = open(&Uri::parse_str("mem://heap").unwrap()).unwrap();
    assert!(io.is_memory() && !io.is_local());
    io.pwrite_utf8(0, "in memory");
    assert_eq!(io.pread_utf8(0, 9).unwrap(), "in memory");
    assert!(io.as_memory().is_some());
    // The uniform cursor streams like a file.
    io.rewind();
    assert_eq!(io.read_utf8(2).unwrap(), "in");
    assert_eq!(io.position(), 2);
}

#[test]
fn open_file_scheme_yields_a_local_handle() {
    let path = std::env::temp_dir().join("yggdryl_open_test.bin");
    let uri = Uri::from_file_path(&path.to_string_lossy());
    let mut io = open(&uri).unwrap();
    assert!(io.is_local() && !io.is_memory());
    // Lazy: writing auto-creates + maps; reading a fresh handle before write is empty.
    io.pwrite_utf8(0, "on disk");
    assert_eq!(io.pread_utf8(0, 7).unwrap(), "on disk");
    io.close();
    // Recover the concrete LocalIO to reach the filesystem graph, then clean up.
    let local = io.into_local().unwrap();
    local.rmfile(true).unwrap();
}

#[test]
fn open_str_parses_path_or_uri() {
    // A mem:// URI string.
    let io = open_str("mem://heap/scratch").unwrap();
    assert!(io.is_memory());
    // A plain (scheme-less) path opens a local handle.
    let tmp = std::env::temp_dir().join("yggdryl_open_str.bin");
    let io2 = open_str(&tmp.to_string_lossy()).unwrap();
    assert!(io2.is_local());
}

#[test]
fn open_rejects_an_unsupported_scheme_with_a_guided_error() {
    let err = open(&Uri::parse_str("https://host/p").unwrap()).unwrap_err();
    let text = err.to_string();
    assert!(text.contains("https") && text.contains("file://") && text.contains("mem://"));
}

#[test]
fn any_io_move_into_relocates_across_schemes() {
    // Move a mem-backed handle's bytes into another mem-backed handle.
    let mut src = AnyIO::memory(yggdryl_core::io::memory::Heap::from_slice(b"cross"));
    let mut dst = open(&Uri::parse_str("mem://heap").unwrap()).unwrap();
    assert_eq!(src.move_into(&mut dst).unwrap(), 5);
    assert_eq!(dst.pread_vec(0, 5), b"cross");
    assert_eq!(src.byte_size(), 0);
}

#[test]
fn any_io_unwrap_mismatch_recovers_or_reports_none() {
    // A mem-backed handle: into_local() returns Err(self) so the caller recovers it; as_local None.
    let io = open(&Uri::parse_str("mem://heap").unwrap()).unwrap();
    assert!(io.as_local().is_none());
    let recovered = io.into_local().unwrap_err(); // Err payload IS the original handle
    assert!(recovered.is_memory());

    // A file-backed handle: into_memory() returns Err(self); as_memory None.
    let uri = Uri::from_file_path(
        &std::env::temp_dir()
            .join("ygg_anyio_mismatch.bin")
            .to_string_lossy(),
    );
    let io2 = open(&uri).unwrap();
    assert!(io2.as_memory().is_none());
    assert!(io2.into_memory().is_err());
}

#[test]
fn open_str_rejects_an_invalid_target() {
    let err = open_str(":not-a-scheme").unwrap_err();
    assert!(err.to_string().contains("not a valid path or URI"));
}
