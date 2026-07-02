//! The `ygg.*` Arrow metadata keys — the single source of truth.
//!
//! Where Arrow lacks a physical type, a yggdryl type anchors on a compatible
//! physical type and restores its semantics through these field-metadata
//! keys. The `ygg.` prefix is reserved: user metadata under it is overwritten
//! on the way out and rejected as unknown on the way in.

/// The prefix reserved for every yggdryl metadata key.
pub const PREFIX: &str = "ygg.";

/// The yggdryl type an anchored physical type restores to (e.g.
/// `"timestamp"` on an `Int64Type` anchoring an extended-unit
/// [`Timestamp`](crate::Timestamp)).
pub const TYPE: &str = "ygg.type";

/// The time unit of an anchored temporal type, as rendered by
/// [`TimeUnitId`](crate::TimeUnitId) (e.g. `"min"`).
pub const TIME_UNIT: &str = "ygg.time_unit";

/// The timezone of an anchored timestamp, when it has one.
pub const TIMEZONE: &str = "ygg.timezone";
