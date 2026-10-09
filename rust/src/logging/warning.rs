//! Deduplicated warnings: what a data door that passes over a value it
//! cannot read says, once.
//!
//! A FIX parse, a market data read and a lifecycle walk never fail on the
//! data they are handed: a fact that does not matter takes its default, an
//! item that cannot stand is excluded, and either is said as a warning
//! naming what was wrong, where, and what was done instead. A stream says
//! the same thing once per row it happens on, so each warning is keyed by
//! where it is raised, what went wrong and the column, tag or kind it is
//! about - never by the value or the row - and logged the first time that
//! key is seen, then again at each tenfold count, with the count.
//!
//! The counting is the tree's own [`Repeats`]: lock-free, keyed by an XXH3
//! hash, bounded at [`Repeats::CAPACITY`] keys. The warnings count in a table
//! of their own, so a logger deduplicating an application's messages never
//! takes room a data warning needs. Past the bound, one line says so and new
//! keys are counted in total only. A key already seen costs one hash and two
//! atomic operations, and builds no text.

use std::hash::Hasher;
use std::sync::atomic::{AtomicU64, Ordering};

use super::repeats::is_tenfold;
use super::{Repeat, Repeats};
use crate::xxhash::Xxh3;

/// The table the data warnings count in.
static WARNINGS: Repeats = Repeats::new();

/// How many warnings arrived for a key the full table could not count.
static OVERFLOW: AtomicU64 = AtomicU64::new(0);

/// The key one warning is deduplicated under: the logger name its site
/// carries - so `yggdryl_fix::market` and `yggdryl.fix.market` are one
/// site, as their records are one logger - then what went wrong and its
/// subject.
fn key(site: &str, what: &str, subject: &str) -> u64 {
    let mut state = Xxh3::new();
    super::facade::write_logger_name(site, &mut state);
    state.write(&[0]);
    for part in [what, subject] {
        state.write(part.as_bytes());
        state.write(&[0]);
    }
    state.finish()
}

/// Warns once about `subject` - the column, tag, field or kind a warning is
/// about - under `what` went wrong at `site`: `detail` is built and logged
/// the first time, naming the value, the location and what was done
/// instead, and the count is logged at each tenfold occurrence after it.
///
/// `what` and `subject` are the key and must not hold the value or the row,
/// or every row would be its own warning.
pub fn warn(
    site: &'static str,
    file: &'static str,
    line: u32,
    what: &'static str,
    subject: &str,
    detail: impl FnOnce() -> String,
) {
    match WARNINGS.count(key(site, what, subject)) {
        Repeat::First => said(
            site,
            file,
            line,
            format_args!(
                "{what} ({subject}): {}; later occurrences are counted rather than repeated",
                detail()
            ),
        ),
        Repeat::Tenfold(count) => {
            said(
                site,
                file,
                line,
                format_args!("{what} ({subject}): seen {count} times"),
            );
        }
        Repeat::Repeated => {}
        Repeat::Untracked => {
            let overflow = OVERFLOW.fetch_add(1, Ordering::Relaxed) + 1;
            if overflow == 1 || is_tenfold(overflow) {
                log::warn!(
                    target: "yggdryl::warning",
                    "{} distinct warnings were logged; {overflow} of other kinds were not",
                    Repeats::CAPACITY
                );
            }
        }
    }
}

/// One warning on the `log` facade, located where `warned!` was written -
/// the module, the file and the line that raised it - rather than here.
fn said(site: &'static str, file: &'static str, line: u32, message: std::fmt::Arguments<'_>) {
    if log::Level::Warn <= log::max_level() {
        log::logger().log(
            &log::Record::builder()
                .args(message)
                .level(log::Level::Warn)
                .target(site)
                .module_path_static(Some(site))
                .file_static(Some(file))
                .line(Some(line))
                .build(),
        );
    }
}

/// [`warn`] at the calling module: `warned!(what, subject, "format", args..)`,
/// the format built only the first time the key is seen.
///
/// Exported for the crates this core is split into, which warn through it:
/// its expansion names [`warn`] through [`crate::implementer`], so it
/// reaches nothing crate-private from the crate that invokes it.
#[macro_export]
#[doc(hidden)]
macro_rules! warned {
    ($what:expr, $subject:expr, $($format:tt)+) => {
        $crate::implementer::warn(
            module_path!(),
            file!(),
            line!(),
            $what,
            $subject,
            || format!($($format)+),
        )
    };
}

#[cfg(feature = "internals")]
#[doc(hidden)]
pub mod internals {
    //! What `rust/tests/logging/warning.rs` pins and a caller cannot reach.

    /// Warns once under `site`, as a door does.
    pub fn warn(site: &'static str, what: &'static str, subject: &str, detail: &str) {
        super::warn(site, file!(), line!(), what, subject, || detail.to_owned());
    }

    /// How many times the warning `what` about `subject` was raised at
    /// `site` - a module path, such as `yggdryl::local::file`, under
    /// whichever of the workspace's crates holds the module - `0` for
    /// one never raised.
    #[must_use]
    pub fn count(site: &str, what: &str, subject: &str) -> u64 {
        super::WARNINGS.seen(super::key(site, what, subject))
    }

    /// Whether `count` is one a repeated warning is logged at again.
    #[must_use]
    pub fn is_tenfold(count: u64) -> bool {
        super::is_tenfold(count)
    }
}
