//! The `IOBase` call counts over what `yggdryl-market` owns, beside the
//! core's own in `rust/tests/iobase_calls.rs`.
//!
//! What every derived surface costs in calls to the handle underneath it.
//!
//! `IOBase` is the one boundary a layer crosses to reach storage, and on a
//! store each crossing is a round trip - so the number of them an operation
//! makes is the property this crate is built around, and these are the tests
//! that hold it. Each states its count exactly rather than as a bound: a read
//! that quietly became two calls is the regression worth catching, and so is
//! an operation that stopped talking to storage at all.
//!
//! [`Counted`] is the instrument. It wraps the byte handle, forwards every
//! call unchanged, and tallies it, so the stack built on top of it is measured
//! rather than argued about. The counts here are what a *layer* asks of
//! storage; how many requests a backend then makes of the network is the object
//! client's own `Stats`, asserted in `rust/tests/s3/`.

#[path = "support/install.rs"]
mod install;
use std::sync::Arc;

use yggdryl::holder::counted::Calls;

/// Run `operation` and assert it cost exactly `expected` in calls.
///
/// The expectation is the tally's own rendering - `read_all_bytes=1`, or
/// `none` - so a failure names the call that appeared as well as the count.
fn costs(what: &str, calls: &Arc<Calls>, expected: &str, operation: impl FnOnce()) {
    calls.reset();
    operation();
    assert_eq!(calls.snapshot().to_string(), expected, "{what}");
}

mod isin_registry {
    use std::sync::Arc;

    use yggdryl::holder::Buffer;
    use yggdryl::holder::counted::Counted;
    use yggdryl::media::IORecordOptions;
    use yggdryl::{IOBase, IOMedia, IOMode, Isin, MimeType};
    use yggdryl_market::{IdType, IsinEntry, IsinRegistry};

    use super::costs;

    /// A registry is read from a holder in exactly the calls one record read
    /// of it makes: the encoding, then the stream, and nothing of its own.
    #[test]
    fn an_isin_registry_reads_a_holder_in_the_calls_of_one_record_read() {
        crate::install::installed();
        let mut registry = IsinRegistry::new();
        registry
            .merge(
                IsinEntry::new(Isin::new("CH0012214059").unwrap())
                    .try_with_code(IdType::Ric, "HOLN.S")
                    .unwrap(),
            )
            .unwrap();
        let mut sink = Buffer::new().with_media_type(MimeType::ARROW_STREAM.into());
        let options = sink
            .record_options()
            .unwrap()
            .with_field(IsinEntry::field());
        sink.write_arrow_reader(
            registry.into_arrow_reader().unwrap(),
            IOMode::Overwrite,
            &options,
        )
        .unwrap();
        let mut source = Buffer::from_bytes(sink.read_all_bytes().unwrap());
        source.set_media_type(MimeType::ARROW_STREAM.into());
        let handle = Counted::new(source);
        let calls = Arc::clone(handle.calls());
        calls.reset();
        let options = handle.record_options().unwrap();
        for batch in handle.read_arrow_reader(&options).unwrap() {
            batch.unwrap();
        }
        let one_read = calls.snapshot().to_string();
        costs("an isin registry read", &calls, &one_read, || {
            let mut registry = IsinRegistry::new();
            assert_eq!(registry.extend_from_handle(&handle).unwrap(), 1);
            assert_eq!(registry.len(), 1);
        });
    }
}
