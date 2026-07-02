//! Crate-internal macro generating the shared shape of a unit-struct time
//! unit, so each unit's file states only what is unique to it.

/// Generates a type-level time unit: the struct itself, its
/// [`TimeUnit`](crate::TimeUnit) implementation tying it to the
/// [`TimeUnitId`](crate::TimeUnitId) variant of the same name, and its
/// render-only `Display` (delegating to the identifier's rendering).
macro_rules! time_unit {
    (
        $(#[$doc:meta])*
        $name:ident
    ) => {
        $(#[$doc])*
        #[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
        #[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
        pub struct $name;

        impl $crate::TimeUnit for $name {
            fn from_unit_id(unit_id: $crate::TimeUnitId) -> Result<Self, $crate::DataTypeError> {
                match unit_id {
                    $crate::TimeUnitId::$name => Ok(Self),
                    other => Err($crate::DataTypeError::TimeUnitMismatch {
                        expected: $crate::TimeUnitId::$name.metadata_value(),
                        actual: other,
                    }),
                }
            }

            fn unit_id(&self) -> $crate::TimeUnitId {
                $crate::TimeUnitId::$name
            }
        }

        impl ::core::fmt::Display for $name {
            fn fmt(&self, f: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result {
                $crate::TimeUnitId::$name.fmt(f)
            }
        }
    };
}

pub(crate) use time_unit;
