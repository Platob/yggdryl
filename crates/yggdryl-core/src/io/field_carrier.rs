//! The **held-field carrier** convention: every concrete `Serie` and `Scalar` holds **its family's
//! own field descriptor** ([`Field`](crate::io::fixed::Field) / `DecimalField` / `TemporalField` /
//! `ByteField` / `FixedSizeField` / `NullField`) as a single member and reads its schema intent —
//! `name`, `nullable` (declared), `metadata`, and the dtype params — **through** it. There is no
//! parallel header type; the field descriptor and the centralized [`Headers`](crate::io::Headers)
//! stay the single source of truth.
//!
//! Three macros keep it DRY:
//! - [`field_setters`] — emitted on a field descriptor (`name: String, nullable: bool, metadata:
//!   Headers`) to give it in-place `set_name` / `set_nullable` / `set_metadata`.
//! - [`field_accessors`] — emitted on a serie/scalar holding a `field:` member, delegating
//!   `name` / `nullable` / `metadata` / `with_*` / `set_*` to it (so field access reads the same on
//!   a `Serie` and a `Scalar`).
//! - [`any_serie_field_forwarding`] — emitted inside an `impl AnySerie` to forward the trait's
//!   header methods to the inherent [`field_accessors`] ones.
//!
//! The held field's `name` / `nullable` / `metadata` are **schema intent**, deliberately excluded
//! from value identity and the byte codec everywhere; only its **dtype params** (precision/scale,
//! unit/tz, width) join the data in identity.

/// Emits in-place `set_name` / `set_nullable` / `set_metadata` on a field descriptor whose members
/// are `name: String`, `nullable: bool`, and `metadata: Headers`. Invoked inside the field type's
/// own module, so it may touch those private members.
macro_rules! field_setters {
    () => {
        /// Sets the column name in place.
        pub fn set_name(&mut self, name: &str) {
            self.name.clear();
            self.name.push_str(name);
        }

        /// Sets the nullability flag in place.
        pub fn set_nullable(&mut self, nullable: bool) {
            self.nullable = nullable;
        }

        /// Replaces the metadata in place (moved in, no clone).
        pub fn set_metadata(&mut self, metadata: $crate::io::Headers) {
            self.metadata = metadata;
        }
    };
}

pub(crate) use field_setters;

/// Emits the public schema-intent accessors on a serie/scalar that holds a `field:` field descriptor
/// member (with `name()` / `nullable()` / `metadata()` reads and `set_name` / `set_nullable` /
/// `set_metadata` in-place setters). Invoked inside the type's inherent `impl`, so `Self` resolves to
/// it. `nullable()` is the **declared** flag; the *effective* nullability a `field()` surfaces adds
/// the value's own null state (`|| has_nulls()` / `|| is_null()`).
macro_rules! field_accessors {
    () => {
        /// The declared name (empty by default) — read from the held field descriptor.
        pub fn name(&self) -> &str {
            self.field.name()
        }

        /// The **declared** nullability flag from the held field (default `false`) — not the
        /// effective nullability a [`field`] surfaces (which also folds in the value's null state).
        pub fn nullable(&self) -> bool {
            self.field.nullable()
        }

        /// The metadata [`Headers`](crate::io::Headers) of the held field.
        pub fn metadata(&self) -> &$crate::io::Headers {
            self.field.metadata()
        }

        /// A fresh value renamed (mutates the held field in place).
        pub fn with_name(mut self, name: impl Into<String>) -> Self {
            self.field.set_name(&name.into());
            self
        }

        /// A fresh value with the given metadata (moved into the held field, no clone).
        pub fn with_metadata(mut self, metadata: $crate::io::Headers) -> Self {
            self.field.set_metadata(metadata);
            self
        }

        /// A fresh value with the given declared nullability.
        pub fn with_nullable(mut self, nullable: bool) -> Self {
            self.field.set_nullable(nullable);
            self
        }

        /// Sets the name in place.
        pub fn set_name(&mut self, name: &str) {
            self.field.set_name(name);
        }

        /// Sets the metadata in place (moved in, no clone).
        pub fn set_metadata(&mut self, metadata: $crate::io::Headers) {
            self.field.set_metadata(metadata);
        }

        /// Sets the declared nullability in place.
        pub fn set_nullable(&mut self, nullable: bool) {
            self.field.set_nullable(nullable);
        }
    };
}

pub(crate) use field_accessors;

/// Emits the [`AnySerie`](crate::io::AnySerie) header methods (`name` / `set_name` / `set_nullable`
/// / `set_metadata`) as thin forwards to the inherent [`field_accessors`] ones. Invoked inside an
/// `impl AnySerie for T` block; each body resolves to the inherent accessor (inherent methods shadow
/// the same-named trait method), so there is no recursion.
macro_rules! any_serie_field_forwarding {
    () => {
        fn name(&self) -> &str {
            self.name()
        }
        fn set_name(&mut self, name: &str) {
            self.set_name(name);
        }
        fn set_nullable(&mut self, nullable: bool) {
            self.set_nullable(nullable);
        }
        fn set_metadata(&mut self, metadata: $crate::io::Headers) {
            self.set_metadata(metadata);
        }
    };
}

pub(crate) use any_serie_field_forwarding;
