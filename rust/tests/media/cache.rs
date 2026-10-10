//! `rust/src/media/cache.rs`: the metadata cache a media wrapper holds, and
//! the time-to-live a closed handle serves it under.
//!
//! Every rule here is stated against instants the test hands the cache - the
//! cache never reads a clock of its own - so none of it needs the `internals`
//! clock the wrappers' suites install.

use std::any::Any;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use yggdryl::media::{CacheTtl, Entry, MediaCache};
use yggdryl::{DataType, Error, Field, StructType};

/// A root with one `id` column, named `name`.
fn root(name: &str) -> Field {
    StructType::from_fields([DataType::Int64.required_field("id")])
        .map(DataType::from)
        .unwrap()
        .required_field(name)
}

/// An entry stating `rows` rows of the `id` root.
fn entry(rows: u64) -> Entry {
    Entry {
        origin: Some(root("row")),
        rows: Some(rows),
        columns: Some(1),
        state: None,
    }
}

/// `milliseconds` after `base`.
fn after(base: Instant, milliseconds: u64) -> Instant {
    base + Duration::from_millis(milliseconds)
}

/// The hash `value` feeds a fresh default hasher.
fn hashed(value: &impl Hash) -> u64 {
    let mut hasher = DefaultHasher::new();
    value.hash(&mut hasher);
    hasher.finish()
}

#[test]
fn a_ttl_of_zero_is_realtime_and_the_default() {
    assert_eq!(CacheTtl::default().millis(), 0);
    assert!(CacheTtl::default().is_realtime());
    assert_eq!(CacheTtl::REALTIME.millis(), 0);
    assert!(CacheTtl::REALTIME.is_realtime());

    assert_eq!(CacheTtl::from(1_500_u64).millis(), 1_500);
    assert_eq!(CacheTtl(1).millis(), 1);
    assert!(!CacheTtl(1).is_realtime());
}

#[test]
fn the_ttl_is_outside_equality_ordering_and_the_hash() {
    fn assert_traits<T: Clone + Copy + Eq + Ord + Hash + Default + std::fmt::Debug>() {}
    assert_traits::<CacheTtl>();

    // It changes when a change is seen, never what is: two values differing
    // only here are the same value.
    for (left, right) in [(0, 1_000), (1_000, 0), (7, 86_400_000), (5, 5)] {
        let (left, right) = (CacheTtl(left), CacheTtl(right));
        assert_eq!(left, right);
        assert_eq!(left.cmp(&right), std::cmp::Ordering::Equal);
        assert_eq!(left.partial_cmp(&right), Some(std::cmp::Ordering::Equal));
        assert_eq!(hashed(&left), hashed(&right));
    }

    // Its `Hash` writes nothing: a hasher fed a TTL is the hasher fed nothing,
    // so a value holding one feeds the same bytes whatever the TTL says.
    assert_eq!(hashed(&CacheTtl(86_400_000)), DefaultHasher::new().finish());
    assert_eq!(hashed(&(1_u8, CacheTtl(9))), hashed(&(1_u8, CacheTtl(0))));
}

#[test]
fn a_ttl_serves_an_entry_strictly_younger_than_itself() {
    let at = Instant::now();
    let ttl = CacheTtl(1_000);

    assert!(ttl.serves(at, at));
    assert!(ttl.serves(at, after(at, 1)));
    assert!(
        ttl.serves(at, after(at, 999)),
        "one millisecond short of the TTL is served"
    );
    assert!(
        !ttl.serves(at, after(at, 1_000)),
        "an entry exactly as old as the TTL is read again"
    );
    assert!(!ttl.serves(at, after(at, 1_001)));
    assert!(!ttl.serves(at, after(at, 3_600_000)));

    // Realtime never serves, not even an entry stamped this instant.
    assert!(!CacheTtl::REALTIME.serves(at, at));
    assert!(!CacheTtl::REALTIME.serves(at, after(at, 1)));
}

#[test]
fn a_new_cache_is_closed_empty_and_serves_nothing() {
    fn assert_send_sync<T: Send + Sync + Default + std::fmt::Debug>() {}
    assert_send_sync::<MediaCache>();

    let cache = MediaCache::new();
    let now = Instant::now();
    assert!(!cache.is_open());
    assert_eq!(cache.generation(), 0);
    assert!(cache.entry(CacheTtl(60_000), now).is_none());
    assert!(cache.entry(CacheTtl::REALTIME, now).is_none());
}

#[test]
fn a_closed_cache_serves_an_entry_only_under_a_ttl_and_only_while_it_is_young() {
    let cache = MediaCache::new();
    let at = Instant::now();
    cache.fill(at, entry(3));

    let ttl = CacheTtl(1_000);
    let served = cache.entry(ttl, after(at, 999)).expect("a young entry");
    assert_eq!(served.rows, Some(3));
    assert_eq!(served.columns, Some(1));
    assert_eq!(served.origin, Some(root("row")));
    assert!(
        cache.entry(ttl, after(at, 1_000)).is_none(),
        "an entry as old as the TTL is not served"
    );
    assert!(
        cache.entry(CacheTtl::REALTIME, at).is_none(),
        "a realtime ask is served by nothing a closed handle holds"
    );
    // The TTL is the ask's: the same entry is served to a longer one.
    assert!(cache.entry(CacheTtl(60_000), after(at, 30_000)).is_some());
}

#[test]
fn an_open_session_serves_its_entry_whatever_the_ttl_until_close() {
    let cache = MediaCache::new();
    let at = Instant::now();
    cache.open();
    assert!(cache.is_open());
    cache.fill(at, entry(5));

    let hour = after(at, 3_600_000);
    for ttl in [CacheTtl::REALTIME, CacheTtl(1), CacheTtl(1_000)] {
        let served = cache.entry(ttl, hour).expect("an open session serves");
        assert_eq!(served.rows, Some(5), "{ttl:?}");
    }

    cache.close();
    assert!(!cache.is_open());
    assert!(
        cache.entry(CacheTtl(60_000), at).is_none(),
        "close drops the entry, whatever the TTL"
    );
    assert!(cache.entry(CacheTtl::REALTIME, at).is_none());
}

#[test]
fn invalidate_drops_the_entry_bumps_the_generation_and_keeps_the_session() {
    let cache = MediaCache::new();
    let at = Instant::now();
    cache.open();
    cache.fill(at, entry(2));
    assert_eq!(cache.generation(), 0);

    cache.invalidate();
    assert_eq!(cache.generation(), 1);
    assert!(cache.is_open(), "an invalidation does not end the session");
    assert!(cache.entry(CacheTtl::REALTIME, at).is_none());

    // Every invalidation counts, an empty cache's included.
    cache.invalidate();
    assert_eq!(cache.generation(), 2);

    // A closed cache drops its young entry the same way.
    let closed = MediaCache::new();
    closed.fill(at, entry(2));
    closed.invalidate();
    assert_eq!(closed.generation(), 1);
    assert!(!closed.is_open());
    assert!(closed.entry(CacheTtl(60_000), at).is_none());
}

#[test]
fn a_fill_replaces_the_entry_whole_and_restamps_it() {
    let cache = MediaCache::new();
    let ttl = CacheTtl(1_000);
    let first = Instant::now();
    cache.fill(first, entry(1));
    let later = after(first, 800);
    cache.fill(
        later,
        Entry {
            origin: None,
            rows: Some(0),
            columns: None,
            state: None,
        },
    );

    // The second entry is stamped at `later`, not carried from the first.
    let served = cache
        .entry(ttl, after(later, 999))
        .expect("restamped by the fill");
    assert!(served.origin.is_none());
    assert_eq!(served.rows, Some(0));
    assert_eq!(served.columns, None);
    assert!(cache.entry(ttl, after(later, 1_000)).is_none());
}

#[test]
fn update_edits_an_empty_cache_into_an_entry_stamped_now() {
    let cache = MediaCache::new();
    let ttl = CacheTtl(1_000);
    let at = Instant::now();
    assert!(cache.entry(ttl, at).is_none());

    cache.update(at, |entry| entry.rows = Some(7));
    let served = cache
        .entry(ttl, after(at, 500))
        .expect("an entry made of nothing but the edit");
    assert_eq!(served.rows, Some(7));
    assert!(served.origin.is_none());
    assert_eq!(served.columns, None);
    assert!(served.state.is_none());
    assert!(cache.entry(ttl, after(at, 1_000)).is_none());
}

#[test]
fn update_edits_what_is_held_in_place_and_restamps_it() {
    let cache = MediaCache::new();
    let ttl = CacheTtl(1_000);
    let first = Instant::now();
    cache.fill(first, entry(4));

    let later = after(first, 900);
    cache.update(later, |entry| {
        entry.rows = entry.rows.map(|rows| rows + 2);
        entry.state = Some(Arc::new(11_u32));
    });
    let served = cache
        .entry(ttl, after(later, 999))
        .expect("the edit restamped the entry");
    assert_eq!(served.rows, Some(6));
    assert_eq!(served.origin, Some(root("row")), "what the edit left alone");
    assert_eq!(served.columns, Some(1));
    assert!(served.state.is_some());
}

#[test]
fn an_update_on_a_closed_cache_is_never_served_to_a_realtime_ask() {
    let cache = MediaCache::new();
    let at = Instant::now();
    cache.update(at, |entry| entry.rows = Some(1));
    assert!(cache.entry(CacheTtl::REALTIME, at).is_none());
    assert!(cache.entry(CacheTtl(1), at).is_some());

    cache.open();
    cache.update(at, |entry| entry.rows = Some(2));
    assert_eq!(
        cache
            .entry(CacheTtl::REALTIME, after(at, 60_000))
            .unwrap()
            .rows,
        Some(2),
        "an open session serves it"
    );
}

#[test]
fn the_state_is_the_mediums_own_object_reached_by_downcast() {
    let cache = MediaCache::new();
    let at = Instant::now();
    let state: Arc<dyn Any + Send + Sync> = Arc::new(String::from("footer"));
    cache.fill(
        at,
        Entry {
            state: Some(Arc::clone(&state)),
            ..Entry::default()
        },
    );

    let served = cache
        .entry(CacheTtl(1_000), at)
        .and_then(|entry| entry.state)
        .expect("the state travels with the entry");
    assert!(Arc::ptr_eq(&served, &state), "shared, never copied");
    assert!(served.clone().downcast::<u32>().is_err());
    assert_eq!(*served.downcast::<String>().unwrap(), "footer");
}

/// A fill that counts how many times it ran, answering `rows` rows.
fn counting(reads: &AtomicUsize, rows: u64) -> yggdryl::Result<Entry> {
    reads.fetch_add(1, Ordering::Relaxed);
    Ok(entry(rows))
}

#[test]
fn get_or_fill_stores_nothing_on_a_realtime_closed_read() {
    let cache = MediaCache::new();
    let reads = AtomicUsize::new(0);
    let at = Instant::now();

    for round in 1..=3 {
        let got = cache
            .get_or_fill(CacheTtl::REALTIME, at, || counting(&reads, 9))
            .unwrap();
        assert_eq!(got.rows, Some(9));
        assert_eq!(reads.load(Ordering::Relaxed), round, "read afresh each ask");
    }
    // Nothing it read could be served to an ask under a TTL either.
    assert!(cache.entry(CacheTtl(60_000), at).is_none());
}

#[test]
fn get_or_fill_stores_under_a_ttl_and_serves_it_until_the_entry_is_as_old_as_the_ttl() {
    let cache = MediaCache::new();
    let ttl = CacheTtl(1_000);
    let reads = AtomicUsize::new(0);
    let at = Instant::now();

    let first = cache.get_or_fill(ttl, at, || counting(&reads, 4)).unwrap();
    assert_eq!(first.rows, Some(4));
    assert_eq!(reads.load(Ordering::Relaxed), 1);

    // Served: the closure is not run, and what it would have said is not seen.
    let warm = cache
        .get_or_fill(ttl, after(at, 999), || counting(&reads, 99))
        .unwrap();
    assert_eq!(warm.rows, Some(4));
    assert_eq!(reads.load(Ordering::Relaxed), 1);

    // Read again at the TTL, and stamped at the ask that read it.
    let again = cache
        .get_or_fill(ttl, after(at, 1_000), || counting(&reads, 5))
        .unwrap();
    assert_eq!(again.rows, Some(5));
    assert_eq!(reads.load(Ordering::Relaxed), 2);
    let warm = cache
        .get_or_fill(ttl, after(at, 1_999), || counting(&reads, 99))
        .unwrap();
    assert_eq!(warm.rows, Some(5));
    assert_eq!(reads.load(Ordering::Relaxed), 2);
}

#[test]
fn get_or_fill_stores_while_open_whatever_the_ttl() {
    let cache = MediaCache::new();
    let reads = AtomicUsize::new(0);
    let at = Instant::now();
    cache.open();

    cache
        .get_or_fill(CacheTtl::REALTIME, at, || counting(&reads, 2))
        .unwrap();
    let hour = after(at, 3_600_000);
    let warm = cache
        .get_or_fill(CacheTtl::REALTIME, hour, || counting(&reads, 99))
        .unwrap();
    assert_eq!(warm.rows, Some(2));
    assert_eq!(reads.load(Ordering::Relaxed), 1);

    cache.close();
    cache
        .get_or_fill(CacheTtl::REALTIME, hour, || counting(&reads, 3))
        .unwrap();
    assert_eq!(reads.load(Ordering::Relaxed), 2, "close dropped the entry");
}

#[test]
fn a_failed_read_is_returned_and_stores_nothing() {
    let cache = MediaCache::new();
    let ttl = CacheTtl(1_000);
    let at = Instant::now();

    let error = cache
        .get_or_fill(ttl, at, || {
            Err(Error::InvalidRecord {
                path: "$.cache".into(),
                reason: "the store is gone".into(),
            })
        })
        .unwrap_err();
    assert!(error.to_string().contains("the store is gone"), "{error}");
    assert!(cache.entry(ttl, at).is_none());

    // The next ask reads, and is not blamed for the one before.
    let reads = AtomicUsize::new(0);
    cache.get_or_fill(ttl, at, || counting(&reads, 1)).unwrap();
    assert_eq!(reads.load(Ordering::Relaxed), 1);
    assert!(cache.entry(ttl, at).is_some());
}

#[test]
fn a_read_that_an_invalidation_overtook_is_returned_but_never_stored() {
    let cache = MediaCache::new();
    let ttl = CacheTtl(1_000);
    let at = Instant::now();

    // The store changed while the read ran, so what it read may describe
    // bytes that are gone: the caller has its answer, the cache keeps none.
    let got = cache
        .get_or_fill(ttl, at, || {
            cache.invalidate();
            Ok(entry(8))
        })
        .unwrap();
    assert_eq!(got.rows, Some(8));
    assert_eq!(cache.generation(), 1);
    assert!(cache.entry(ttl, at).is_none());
}

#[test]
fn a_cache_keeps_what_it_learns_while_open_or_under_a_ttl() {
    let cache = MediaCache::new();
    assert!(!cache.keeps(CacheTtl::REALTIME));
    assert!(cache.keeps(CacheTtl(1)));
    cache.open();
    assert!(cache.keeps(CacheTtl::REALTIME));
    cache.close();
    assert!(!cache.keeps(CacheTtl::REALTIME));
}

#[test]
fn add_lays_a_fact_on_the_served_entry_and_keeps_its_stamp() {
    let cache = MediaCache::new();
    let ttl = CacheTtl(1_000);
    let t0 = Instant::now();
    let mut stated = entry(3);
    stated.rows = None;
    cache.fill(t0, stated);

    // A count learned later is added to what the entry already states...
    cache.add(ttl, after(t0, 400), |entry| entry.rows = Some(3));
    let served = cache.entry(ttl, after(t0, 999)).expect("served");
    assert_eq!(served.rows, Some(3));
    assert_eq!(served.origin, Some(root("row")));
    // ...and the entry's age still counts from the reading it was.
    assert!(cache.entry(ttl, after(t0, 1_000)).is_none());
}

#[test]
fn add_edits_nothing_where_no_entry_is_served() {
    let cache = MediaCache::new();
    let t0 = Instant::now();
    // An empty cache is not made an entry: a fact alone never stands for a
    // shape nobody read.
    cache.add(CacheTtl(1_000), t0, |entry| entry.rows = Some(1));
    assert!(cache.entry(CacheTtl(1_000), t0).is_none());

    // An entry too old to serve is not edited either.
    cache.fill(t0, entry(3));
    cache.add(CacheTtl(1_000), after(t0, 1_000), |entry| {
        entry.rows = Some(7);
    });
    assert_eq!(
        cache
            .entry(CacheTtl(5_000), after(t0, 1_000))
            .and_then(|entry| entry.rows),
        Some(3)
    );
}
