//! The nine **temporal column concepts+widths** — the `Date32Kind` … `Duration64Kind`
//! [`TemporalBacking`] markers and their `*Type` / `*Field` / `*Scalar` / `*Serie` aliases over the
//! generic columnar types. Each marker pins its value type, [`DataTypeId`], physical width, unit /
//! timezone capability, default unit, and admitted units.

use super::{
    Date32, Date64, Duration32, Duration64, TemporalBacking, TemporalField, TemporalNative,
    TemporalScalar, TemporalSerie, TemporalType, Time32, Time64, TimeUnit, Ts32, Ts64, Ts96,
};
use crate::io::DataTypeId;

/// Declares one temporal concept+width marker (implementing [`TemporalBacking`]) and its four
/// columnar aliases, mirroring the sibling markers file-for-line.
macro_rules! temporal_width {
    (
        $Kind:ident, $Native:ty, $name:literal, $width:literal, $id:ident,
        carries_unit = $cu:literal, carries_tz = $ctz:literal, default = $du:ident,
        allows = |$u:ident| $allows:expr,
        $Type:ident, $Field:ident, $Scalar:ident, $Serie:ident
    ) => {
        #[doc = concat!("The `", $name, "` temporal column marker.")]
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
        pub struct $Kind;

        impl TemporalBacking for $Kind {
            type Native = $Native;
            const NAME: &'static str = $name;
            const WIDTH: usize = $width;
            const TYPE_ID: DataTypeId = DataTypeId::$id;
            const CARRIES_UNIT: bool = $cu;
            const CARRIES_TZ: bool = $ctz;
            const DEFAULT_UNIT: TimeUnit = TimeUnit::$du;
            fn allows_unit($u: TimeUnit) -> bool {
                $allows
            }
        }

        // Lock the physical width to the value type's own count width.
        const _: () = assert!(
            $width == <$Native as TemporalNative>::WIDTH,
            "TemporalBacking::WIDTH must equal its Native's TemporalNative::WIDTH"
        );

        #[doc = concat!("The `", $name, "` columnar descriptor (resolution + timezone).")]
        pub type $Type = TemporalType<$Kind>;
        #[doc = concat!("A named, nullable `", $name, "` column descriptor.")]
        pub type $Field = TemporalField<$Kind>;
        #[doc = concat!("One nullable `", $name, "` value carried with its column resolution + timezone.")]
        pub type $Scalar = TemporalScalar<$Kind>;
        #[doc = concat!("A nullable column of `", $name, "` values.")]
        pub type $Serie = TemporalSerie<$Kind>;
    };
}

temporal_width!(
    Date32Kind,
    Date32,
    "date32",
    4,
    Date32,
    carries_unit = false,
    carries_tz = false,
    default = Day,
    allows = |u| matches!(u, TimeUnit::Day),
    Date32Type,
    Date32Field,
    Date32Scalar,
    Date32Serie
);

temporal_width!(
    Date64Kind,
    Date64,
    "date64",
    8,
    Date64,
    carries_unit = false,
    carries_tz = false,
    default = Millisecond,
    allows = |u| matches!(u, TimeUnit::Millisecond),
    Date64Type,
    Date64Field,
    Date64Scalar,
    Date64Serie
);

temporal_width!(
    Time32Kind,
    Time32,
    "time32",
    4,
    Time32,
    carries_unit = true,
    carries_tz = false,
    default = Second,
    allows = |u| matches!(u, TimeUnit::Second | TimeUnit::Millisecond),
    Time32Type,
    Time32Field,
    Time32Scalar,
    Time32Serie
);

temporal_width!(
    Time64Kind,
    Time64,
    "time64",
    8,
    Time64,
    carries_unit = true,
    carries_tz = false,
    default = Microsecond,
    allows = |u| matches!(u, TimeUnit::Microsecond | TimeUnit::Nanosecond),
    Time64Type,
    Time64Field,
    Time64Scalar,
    Time64Serie
);

temporal_width!(
    Ts32Kind,
    Ts32,
    "ts32",
    4,
    Ts32,
    carries_unit = true,
    carries_tz = true,
    default = Nanosecond,
    allows = |u| !u.is_calendar(),
    Ts32Type,
    Ts32Field,
    Ts32Scalar,
    Ts32Serie
);

temporal_width!(
    Ts64Kind,
    Ts64,
    "ts64",
    8,
    Ts64,
    carries_unit = true,
    carries_tz = true,
    default = Nanosecond,
    allows = |u| !u.is_calendar(),
    Ts64Type,
    Ts64Field,
    Ts64Scalar,
    Ts64Serie
);

temporal_width!(
    Ts96Kind,
    Ts96,
    "ts96",
    12,
    Ts96,
    carries_unit = true,
    carries_tz = true,
    default = Nanosecond,
    allows = |u| !u.is_calendar(),
    Ts96Type,
    Ts96Field,
    Ts96Scalar,
    Ts96Serie
);

temporal_width!(
    Duration32Kind,
    Duration32,
    "duration32",
    4,
    Duration32,
    carries_unit = true,
    carries_tz = false,
    default = Nanosecond,
    allows = |u| !u.is_calendar(),
    Duration32Type,
    Duration32Field,
    Duration32Scalar,
    Duration32Serie
);

temporal_width!(
    Duration64Kind,
    Duration64,
    "duration64",
    8,
    Duration64,
    carries_unit = true,
    carries_tz = false,
    default = Nanosecond,
    allows = |u| !u.is_calendar(),
    Duration64Type,
    Duration64Field,
    Duration64Scalar,
    Duration64Serie
);
