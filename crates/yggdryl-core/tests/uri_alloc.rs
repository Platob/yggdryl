//! Deterministic allocation budgets for the URI value types — the fast, build-independent
//! half of "validate both time and memory". Allocation *counts* do not depend on the
//! optimizer or the machine, so unlike wall-clock timing they can be asserted exactly and
//! run in milliseconds, guarding the zero-copy accessors and the at-most-one-copy codec
//! against regressions. (Throughput lives in the `uri` bench.)
//!
//! This file is its own test binary with its own counting global allocator, and holds a
//! **single** `#[test]` so nothing else allocates on another thread while a region is
//! measured.

use std::alloc::{GlobalAlloc, Layout, System};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};

use yggdryl_core::uri::Uri;

struct Counting;
static ALLOCS: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let ptr = System.alloc(layout);
        if !ptr.is_null() {
            ALLOCS.fetch_add(1, Relaxed);
        }
        ptr
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        System.dealloc(ptr, layout);
    }
}

#[global_allocator]
static GLOBAL: Counting = Counting;

/// Total allocations `op` makes over `iters` runs, after one warm-up run so any one-time
/// initialization stays outside the measured window.
fn allocs_over(iters: usize, mut op: impl FnMut()) -> usize {
    op();
    let before = ALLOCS.load(Relaxed);
    for _ in 0..iters {
        op();
    }
    ALLOCS.load(Relaxed) - before
}

fn hash_of<T: Hash>(value: &T) -> u64 {
    let mut hasher = DefaultHasher::new();
    value.hash(&mut hasher);
    hasher.finish()
}

#[test]
fn allocation_budgets() {
    let iters = 1000;
    let uri = Uri::parse("https://user:pw@example.com:8080/a/b/c.tar.gz?q=1#frag").unwrap();

    // Zero-copy accessors borrow from the `Uri` — they must allocate nothing at all.
    let borrow = allocs_over(iters, || {
        let _ = (
            uri.scheme(),
            uri.host(),
            uri.path(),
            uri.name(),
            uri.extension(),
        );
        let _ = (
            uri.user(),
            uri.password(),
            uri.port(),
            uri.query(),
            uri.fragment(),
        );
    });
    assert_eq!(
        borrow, 0,
        "accessors must be zero-copy (got {borrow} allocs)"
    );

    // The effective-endpoint accessors are derived on read: `default_port` / `port_or_default`
    // are a table scan returning a `u16`, `host_is_ipv6` a `bool`, and `host_unbracketed` a
    // borrow — none may allocate.
    let ipv6 = Uri::parse("https://[2001:db8::1]/status").unwrap();
    let endpoint = allocs_over(iters, || {
        let _ = (
            uri.default_port(),
            uri.port_or_default(),
            ipv6.host_is_ipv6(),
            ipv6.host_unbracketed(),
        );
    });
    assert_eq!(
        endpoint, 0,
        "endpoint accessors must be zero-copy (got {endpoint} allocs)"
    );

    // The combinators keep the at-most-one-copy discipline. `copy` is a plain clone; joining
    // a clean segment must add EXACTLY ONE allocation over that clone — the single pre-sized
    // joined path. That also proves the `Cow` back-slash normalization *borrows* on a clean
    // segment (a `String`-returning normalize would show up as a second allocation here).
    let copy_allocs = allocs_over(iters, || {
        let _ = uri.copy();
    });
    let join_allocs = allocs_over(iters, || {
        let _ = uri.joinpath("segment");
    });
    assert_eq!(
        join_allocs,
        copy_allocs + iters,
        "joinpath must add exactly one allocation over a copy (got {join_allocs} vs {copy_allocs}+{iters})"
    );

    // `merge_with` clones only components, never re-parses, so overlaying a default (empty)
    // URI allocates exactly as much as a copy.
    let merge_allocs = allocs_over(iters, || {
        let _ = uri.merge_with(&Uri::default());
    });
    assert_eq!(
        merge_allocs, copy_allocs,
        "merge_with(default) must allocate exactly like a copy (got {merge_allocs} vs {copy_allocs})"
    );

    // A clean POSIX `from_path` still owns exactly one string: the `Cow` normalization borrows,
    // so there is no throwaway allocation before the final owned path.
    let posix_from_path = allocs_over(iters, || {
        let _ = Uri::from_path("/usr/local/share/data/set.csv");
    });
    assert_eq!(
        posix_from_path, iters,
        "from_path of a clean POSIX path must allocate exactly once (got {posix_from_path})"
    );

    // The byte codec is at-most-one-copy: exactly one allocation per `serialize_bytes`,
    // regardless of URI length (the buffer is pre-sized).
    let serialize = allocs_over(iters, || {
        let _ = uri.serialize_bytes();
    });
    assert_eq!(
        serialize, iters,
        "serialize_bytes must allocate exactly once per call"
    );

    // Hashing streams into the hasher — no `String`, so zero allocations.
    let hash = allocs_over(iters, || {
        let _ = hash_of(&uri);
    });
    assert_eq!(
        hash, 0,
        "Hash must not allocate (got {hash} allocs over {iters})"
    );

    // `from_path` owns one normalized path string: exactly one allocation.
    let from_path = allocs_over(iters, || {
        let _ = Uri::from_path(r"C:\Users\x\a.txt");
    });
    assert_eq!(
        from_path, iters,
        "from_path must allocate exactly once per call"
    );

    // The streaming hash must reproduce the canonical string's own hash exactly — bytes
    // then a `0xff` terminator — including the pathological case where a scheme-less path
    // and a scheme+path render to the same string.
    for s in [
        "https://user:pw@example.com:8080/a/b.txt?q=1#frag",
        "/relative/path",
        "mailto:person@example.com",
        "a:b", // scheme "a" + path "b" — renders "a:b", same as the bare path "a:b"
    ] {
        let uri = Uri::parse(s).unwrap();
        assert_eq!(
            hash_of(&uri),
            hash_of(&uri.to_string()),
            "streaming hash must equal the canonical string's hash for {s:?}"
        );
    }

    // Query-parameter access. `query_param` / `has_query_param` borrow into the query and
    // return a `&str` / `bool` — zero allocation.
    let q = Uri::parse("http://h/p?a=1&b=2&c=3&a=4").unwrap();
    let read = allocs_over(iters, || {
        let _ = q.query_param("c");
        let _ = q.has_query_param("b");
    });
    assert_eq!(
        read, 0,
        "query_param / has_query_param must be zero-copy (got {read})"
    );

    // Decoding a value with nothing to decode borrows it — zero allocation. (`c=3` and the
    // clean lookup key both stay borrowed.)
    let decode_clean = allocs_over(iters, || {
        let _ = q.query_param_decoded("c");
    });
    assert_eq!(
        decode_clean, 0,
        "query_param_decoded of a clean value must not allocate (got {decode_clean})"
    );

    // The multi-value and map views each build one pre-sized `Vec`.
    let all = allocs_over(iters, || {
        let _ = q.query_param_all("a");
    });
    assert_eq!(
        all, iters,
        "query_param_all must pre-size to one allocation"
    );
    let params = allocs_over(iters, || {
        let _ = q.query_params();
    });
    assert_eq!(
        params, iters,
        "query_params must pre-size to one allocation"
    );

    // A write rebuilds the query in exactly one allocation.
    let mut set = Uri::parse("http://h/p?a=1&b=2").unwrap();
    let writes = allocs_over(iters, || {
        set.set_query_param("a", "1");
    });
    assert_eq!(
        writes, iters,
        "set_query_param must rebuild with exactly one allocation"
    );

    // Removing an absent key is a no-op — no rebuild, no allocation.
    let mut noop = Uri::parse("http://h/p?a=1&b=2").unwrap();
    let removes_absent = allocs_over(iters, || {
        let _ = noop.remove_query_param("zzz");
    });
    assert_eq!(
        removes_absent, 0,
        "removing an absent param must not allocate"
    );

    // A bulk update rebuilds once with a small **constant** allocation count (the dedup Vec,
    // the bookkeeping Vec, and the output) — independent of the number of params, unlike
    // calling `set_query_param` in a loop (one full rebuild each).
    let mut bulk = Uri::parse("http://h/p?a=1&b=2").unwrap();
    let bulk_allocs = allocs_over(iters, || {
        bulk.set_query_params(&[("a", "9"), ("c", "7"), ("d", "0")]);
    });
    assert_eq!(
        bulk_allocs,
        3 * iters,
        "set_query_params rebuilds in a constant 3 allocations"
    );

    // Normalizing a small query rebuilds in two allocations (the token list + the output;
    // the sort is in-place for a small slice).
    let mut norm = Uri::parse("http://h/p?c=3&a=1&b=2").unwrap();
    let norm_allocs = allocs_over(iters, || {
        norm.normalize_query();
    });
    assert_eq!(
        norm_allocs,
        2 * iters,
        "normalize_query rebuilds in two allocations"
    );
}
