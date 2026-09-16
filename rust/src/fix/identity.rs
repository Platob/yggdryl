//! The one boundary settling clocks and named-content identity.

use std::sync::Arc;

use crate::hashing::{txhash::TxHash, xxhash};
use crate::types::{Bytes, BytesLayout, BytesParameters};
use crate::{DataType, Digest, Error, Field, Result, Scalar, TimeUnit, Timezone};

use super::schema::{CLOCK_DATATYPE, FIXENTRIES_COLUMN};
use super::{
    CODE_TAG_NAME, CREATEDAT_TAG_NAME, FixRegistry, MSGHASH_TAG_NAME, MSGPHASH_TAG_NAME,
    SNAPSHOTAT_TAG_NAME, UPDATEDAT_TAG_NAME,
};

/// The bytes every FIX identity column holds.
///
/// Sixteen of them, big-endian, with no version or variant bit: a lake engine
/// reads `fixed[16]` everywhere and `uuid` nowhere consistently, so the FIX
/// layer states plain bytes and keeps every identity semantic (decisions
/// 19-28) exactly as it was.
pub(super) type Identity = [u8; IDENTITY_WIDTH as usize];

/// The one width, as the datatype spells it.
pub(super) const IDENTITY_WIDTH: u32 = 16;

/// The datatype every FIX identity column declares.
pub(super) const IDENTITY_DATATYPE: DataType = match std::num::NonZeroU32::new(IDENTITY_WIDTH) {
    Some(width) => {
        DataType::Bytes(BytesParameters::new(BytesLayout::FixedSizeBinary).with_bound(width))
    }
    // The width above is a literal greater than zero.
    None => DataType::binary(),
};

/// One identity as the value a row carries, under the fixed layout so it
/// types as [`IDENTITY_DATATYPE`] rather than as unbounded bytes.
pub(super) fn identity_scalar(bytes: Identity) -> Scalar {
    let parameters = IDENTITY_DATATYPE
        .bytes_parameters()
        .expect("the identity datatype is a byte layout");
    Scalar::Bytes(
        Bytes::new(bytes)
            .try_with_parameters(parameters)
            .expect("sixteen bytes under the sixteen-byte layout"),
    )
}

/// The sixteen bytes a stated identity holds, refusing every other value at
/// the column that stated it.
pub(super) fn stated_identity(name: &str, held: &Scalar) -> Result<Identity> {
    let bytes = match held {
        Scalar::Bytes(held) if held.fixed() == Some(IDENTITY_WIDTH) => held.as_bytes(),
        held => {
            return Err(refused(
                name,
                format_args!("{IDENTITY_DATATYPE} or null"),
                crate::text::elide_display(&format_args!("{held:?}")),
            ));
        }
    };
    Ok(Identity::try_from(bytes).expect("the fixed layout proved the width"))
}

/// The sixteen bytes as the lowercase hex a name spells them with.
///
/// A chain code scopes an identifier under the instrument it reached
/// (decision 22); the scope is now bytes rather than an RFC identifier, so
/// the name carries the bytes the way every binary literal in this crate is
/// written - thirty-two lowercase hex digits, no separators.
pub(super) struct IdentityText(pub(super) Identity);

impl std::fmt::Display for IdentityText {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for byte in self.0 {
            write!(formatter, "{byte:02x}")?;
        }
        Ok(())
    }
}

/// The replay bundle, in one fixed internal order unrelated to column order.
#[derive(Clone, Copy)]
pub(super) enum Role {
    Updated,
    Created,
    MsgHash,
    Persistent,
    Code,
    Snapshot,
    Sending,
}

impl Role {
    pub(super) const ALL: [Self; 7] = [
        Self::Updated,
        Self::Created,
        Self::MsgHash,
        Self::Persistent,
        Self::Code,
        Self::Snapshot,
        Self::Sending,
    ];

    pub(super) const fn identity(self) -> (i32, &'static str) {
        match self {
            Self::Updated => UPDATEDAT_TAG_NAME,
            Self::Created => CREATEDAT_TAG_NAME,
            Self::MsgHash => MSGHASH_TAG_NAME,
            Self::Persistent => MSGPHASH_TAG_NAME,
            Self::Code => CODE_TAG_NAME,
            Self::Snapshot => SNAPSHOTAT_TAG_NAME,
            Self::Sending => (52, "sendingtime"),
        }
    }

    fn of(tag: i32) -> Option<Self> {
        Self::ALL.into_iter().find(|role| role.identity().0 == tag)
    }

    fn dtype(self) -> DataType {
        match self {
            Self::MsgHash | Self::Persistent => IDENTITY_DATATYPE,
            Self::Code => DataType::utf8(),
            _ => CLOCK_DATATYPE,
        }
    }
}

pub(super) fn is_mandatory(tag: i32) -> bool {
    // `snapshotat` is a role - replay reads it back where a snapshot wrote
    // one - without being a value every message has, so it is the one role
    // whose column is nullable.
    Role::of(tag).is_some_and(|role| !matches!(role, Role::Snapshot))
}

/// Whether a column is one the replay bundle holds, whatever it is spelled.
///
/// [`is_mandatory`] says the *value* must be there; this says the *column*
/// must. They differ on `snapshotat` alone: a row that is not a snapshot
/// carries the column empty, and a row that cannot carry it at all could not
/// replay a snapshot that was taken. So a holder is never removed and is
/// always reached by its tag under whatever name it was renamed to, while
/// only the values a message always has refuse a null.
pub(super) fn is_held(tag: i32) -> bool {
    Role::of(tag).is_some()
}

/// Optional TransactTime uses the same critical clock boundary as the bundle.
pub(super) fn required_dtype(tag: i32) -> Option<DataType> {
    Role::of(tag)
        .map(Role::dtype)
        .or_else(|| (tag == 60).then_some(CLOCK_DATATYPE))
}

pub(super) fn refused(
    name: &str,
    expected: impl std::fmt::Display,
    actual: impl std::fmt::Display,
) -> Error {
    Error::InvalidRecord {
        path: crate::path::Path::root().field(name).render().into(),
        reason: crate::text::expected_got(expected, crate::text::elide_display(&actual)),
    }
}

pub(super) fn validate_field(field: &Field, tag: i32) -> Result<()> {
    if let Some(expected) = required_dtype(tag) {
        if field.dtype() != &expected || field.as_fix().counter()?.is_some() {
            return Err(refused(field.name(), expected, field.dtype()));
        }
    }
    Ok(())
}

/// Resolve once; an explicit tag never falls through to an unrelated name.
pub(super) fn resolve_tag(field: &Field, registry: &FixRegistry) -> Result<Option<i32>> {
    let explicit = field.as_fix().tag()?;
    let named = super::field::parse_tag(field.name()).or_else(|| {
        registry
            .get_message_field_by_name(field.name())
            .and_then(|known| registry.identity_of(known).map(|(tag, _)| tag))
    });
    if let (Some(tag), Some(claim)) = (explicit, named) {
        if tag != claim && is_held(claim) {
            return Err(refused(
                field.name(),
                format_args!("FIX tag {claim} claimed by its name"),
                format_args!("explicit tag {tag}"),
            ));
        }
    }
    Ok(explicit.or(named))
}

/// Whether a column stands outside the content a message is identified by.
///
/// Three of them are the identity itself and would hash themselves:
/// `msghash` is what is being computed, and `updatedat` and `createdat` are the
/// clocks it is computed against. `sourceurl` is outside for the opposite
/// reason: where a line was read from is a fact about the capture, not about
/// the message. The same message read out of a re-cut file, a replayed
/// archive or a second copy of one day's log is the same message, and it must
/// digest to the same sixteen bytes in all of them.
///
/// `recordedat` is outside for the same reason as `sourceurl` and beside it:
/// *when* a capture wrote the line down is a fact about the capture, not
/// about the message. The same message re-cut, replayed or copied is recorded
/// at a second instant and must still digest to the same sixteen bytes.
///
/// `nofixentries` is outside because the record it counts is: the arrival
/// record is the message rather than a reading of it, and a count of it is
/// the same fact one integer shorter. A row projected without the record
/// would otherwise identify differently from the row that carries it.
fn outside_content(tag: i32) -> bool {
    [
        MSGHASH_TAG_NAME.0,
        UPDATEDAT_TAG_NAME.0,
        CREATEDAT_TAG_NAME.0,
        super::SOURCEURL_TAG_NAME.0,
        super::RECORDEDAT_TAG_NAME.0,
        super::NOFIXENTRIES_TAG_NAME.0,
    ]
    .contains(&tag)
}

/// Part of the shared FIX column plan, never a second schema.
pub(super) struct Plan {
    roles: [Option<usize>; 7],
    order: Box<[usize]>,
}

impl Plan {
    pub(super) fn new(
        schema: &Field,
        tags: impl Iterator<Item = Option<i32>>,
        registry: &FixRegistry,
    ) -> Result<Self> {
        for role in Role::ALL {
            let (tag, name) = role.identity();
            let field = registry.get_field_by_tag(tag).ok_or_else(|| {
                refused(
                    name,
                    "a registered mandatory definition",
                    "missing definition",
                )
            })?;
            validate_field(field, tag)?;
        }
        let mut roles = [None; 7];
        let mut order = Vec::with_capacity(schema.fields().len());
        for (index, (field, tag)) in schema.fields().iter().zip(tags).enumerate() {
            if let Some(tag) = tag {
                validate_field(field, tag)?;
                if let Some(role) = Role::of(tag) {
                    if roles[role as usize].replace(index).is_some() {
                        return Err(refused(
                            field.name(),
                            "one mandatory field holder",
                            "duplicate holder",
                        ));
                    }
                }
            }
            if field.name() != FIXENTRIES_COLUMN && !tag.is_some_and(outside_content) {
                order.push(index);
            }
        }
        order.sort_unstable_by(|left, right| {
            schema.fields()[*left]
                .name()
                .cmp(schema.fields()[*right].name())
        });
        Ok(Self {
            roles,
            order: order.into_boxed_slice(),
        })
    }

    pub(super) fn index(&self, role: Role) -> Result<usize> {
        self.roles[role as usize].ok_or_else(|| {
            refused(
                role.identity().1,
                "a mandatory replay field",
                "missing field",
            )
        })
    }

    pub(super) fn require_bundle(&self, schema: &Field) -> Result<()> {
        for role in Role::ALL {
            // Every role's column is required - the holder is what replay
            // reads back - but `snapshotat` is the one whose value is not:
            // it says this row is a reading the lifecycle took, and an
            // ordinary message is not one.
            let at = self.index(role)?;
            if matches!(role, Role::Snapshot) {
                continue;
            }
            if schema.fields()[at].is_nullable() {
                return Err(refused(
                    schema.fields()[at].name(),
                    "a non-null mandatory field",
                    "nullable declaration",
                ));
            }
        }
        Ok(())
    }

    /// Presence is checked before Struct canonicalization can supply defaults.
    pub(super) fn require_values(&self, schema: &Field, row: &Scalar) -> Result<()> {
        self.require_bundle(schema)?;
        for role in Role::ALL {
            let at = self.index(role)?;
            let name = schema.fields()[at].name();
            let value = if let Some(record) = row.as_record() {
                record.get(name)
            } else {
                row.get(at)
            };
            match value {
                Some(value) if !value.is_null() => validate_value(name, &role.dtype(), value)?,
                // The snapshot clock is empty on every row no snapshot took,
                // so what replay needs from it is the holder and not a value.
                Some(_) if matches!(role, Role::Snapshot) => {}
                _ => {
                    return Err(refused(
                        name,
                        "a present non-null replay value",
                        "missing or null value",
                    ));
                }
            }
        }
        Ok(())
    }

    /// Recompute only derived identities; supplied assertions apply to this candidate.
    pub(super) fn finalize(
        &self,
        schema: &Field,
        values: &mut [Scalar],
        assertions: Assertions,
    ) -> Result<Hard> {
        if values.len() != schema.fields().len() {
            return Err(refused(schema.name(), schema.fields().len(), values.len()));
        }
        for role in Role::ALL {
            let at = self.index(role)?;
            if matches!(role, Role::MsgHash | Role::Persistent | Role::Snapshot)
                && values[at].is_null()
            {
                continue;
            }
            validate_value(schema.fields()[at].name(), &role.dtype(), &values[at])?;
        }
        let code_at = self.index(Role::Code)?;
        let code = values[code_at].as_str().ok_or_else(|| {
            refused(
                schema.fields()[code_at].name(),
                "UTF-8 code",
                values[code_at].kind(),
            )
        })?;
        let persistent = identity_scalar(persistent_identity(code));
        let persistent_at = self.index(Role::Persistent)?;
        assert_identity(
            &schema.fields()[persistent_at],
            &values[persistent_at],
            &persistent,
            assertions.persistent,
        )?;
        values[persistent_at] = persistent;
        let updated_at = self.index(Role::Updated)?;
        let nanos = values[updated_at]
            .temporal_count_at(TimeUnit::Nanosecond)
            .ok_or_else(|| {
                refused(
                    UPDATEDAT_TAG_NAME.1,
                    "signed nanoseconds",
                    values[updated_at].kind(),
                )
            })?;
        let mut state = xxhash::Xxh64::new();
        xxhash::write_named_bytes(
            &mut state,
            self.order
                .iter()
                .map(|at| (schema.fields()[*at].name(), &values[*at])),
            0,
        );
        let msghash = identity_scalar(
            TxHash::new_in(
                nanos,
                TimeUnit::Nanosecond,
                Digest::new(crate::DigestAlgorithm::Xxh64, u128::from(state.as_u64())),
            )?
            .into_ordered_bytes()?,
        );
        let msghash_at = self.index(Role::MsgHash)?;
        assert_identity(
            &schema.fields()[msghash_at],
            &values[msghash_at],
            &msghash,
            assertions.msghash,
        )?;
        values[msghash_at] = msghash;
        Ok(Hard {
            updatedat: values[updated_at].clone(),
            createdat: values[self.index(Role::Created)?].clone(),
            msghash: values[msghash_at].clone(),
            msgphash: values[persistent_at].clone(),
        })
    }
}

#[derive(Clone, Copy, Default)]
pub(super) struct Assertions {
    pub(super) msghash: bool,
    pub(super) persistent: bool,
}

impl Assertions {
    pub(super) const STATED: Self = Self {
        msghash: true,
        persistent: true,
    };
}

/// Moved into the message's four direct fields after the row is proven.
pub(super) struct Hard {
    pub(super) updatedat: Scalar,
    pub(super) createdat: Scalar,
    pub(super) msghash: Scalar,
    pub(super) msgphash: Scalar,
}

/// The event chain's sixteen bytes: the XXH3-128 of the exact code bytes,
/// big-endian, with nothing masked out of them.
pub(super) fn persistent_identity(code: &str) -> Identity {
    xxhash::xxh128(code.as_bytes()).to_be_bytes()
}

fn assert_identity(field: &Field, stated: &Scalar, computed: &Scalar, assert: bool) -> Result<()> {
    if assert && !stated.is_null() && stated != computed {
        return Err(refused(
            field.name(),
            format_args!("{computed:?}"),
            format_args!("{stated:?}"),
        ));
    }
    Ok(())
}

pub(super) fn validate_value(name: &str, dtype: &DataType, value: &Scalar) -> Result<()> {
    let exact = match dtype {
        dtype if dtype == &IDENTITY_DATATYPE => {
            matches!(value, Scalar::Bytes(held) if held.fixed() == Some(IDENTITY_WIDTH))
        }
        dtype if dtype == &CLOCK_DATATYPE => {
            matches!(value.as_datetime64(), Some((_, TimeUnit::Nanosecond, zone)) if *zone == Timezone::UTC)
        }
        _ => matches!(value, Scalar::String(_)),
    };
    if exact {
        Ok(())
    } else {
        Err(refused(name, dtype, format_args!("{value:?}")))
    }
}

/// Intake alone may fill absent roles. Existing values have already been typed.
pub(super) fn fresh(
    registry: &Arc<FixRegistry>,
    mut field: Field,
    row: Scalar,
    fallback: Option<&Scalar>,
    initial: &super::schema::Columns,
) -> Result<(Field, Scalar, Hard)> {
    let mut roles = initial.identity.roles;
    let missing = roles.iter().filter(|at| at.is_none()).count();
    let capacity = field.fields().len() + missing;
    let mut members = Vec::with_capacity(capacity);
    members.extend_from_slice(field.fields());
    let held = row
        .as_sequence()
        .ok_or_else(|| refused(field.name(), "a canonical Struct row", row.kind()))?;
    let mut values = Vec::with_capacity(capacity);
    values.extend_from_slice(held);
    let source = |role: Role| {
        roles[role as usize]
            .and_then(|at| values.get(at))
            .filter(|value| !value.is_null())
            .cloned()
    };
    let sending = source(Role::Sending)
        .or_else(|| fallback.filter(|value| !value.is_null()).cloned())
        .map_or_else(now, Ok)?;
    validate_value("sendingtime", &CLOCK_DATATYPE, &sending)?;
    let stated_clock = |tag: i32| {
        initial
            .iter()
            .position(|column| column.tag == Some(tag))
            .and_then(|at| values.get(at))
            .filter(|value| !value.is_null())
            .cloned()
    };
    let transact = stated_clock(60);
    // The instant the event happened, which is what the clocks below default
    // to. It is not the same thing as the `snapshotat` column: this is always
    // knowable, and the column says something narrower.
    let event = source(Role::Snapshot)
        .or(transact)
        .unwrap_or_else(|| sending.clone());
    // `snapshotat` is a *stated* value or nothing. A snapshot is a reading
    // the lifecycle takes of a chain at a grid instant; an ordinary message
    // is not one, and filling the column on every message made "this row is
    // a snapshot" unanswerable from the row. The lifecycle stamps it where
    // it takes one; intake only keeps what the message itself said.
    let snapshot = source(Role::Snapshot).unwrap_or(Scalar::Null);
    let updated = source(Role::Updated).unwrap_or_else(|| event.clone());
    // What a message says about its own creation, strongest first: a stated
    // `createdat`, then `OrigSendingTime(122)`, then the snapshot instant.
    //
    // 122 is in the middle because of what it means. A resend carries the
    // instant the original was sent, and that original is when this message
    // came into being - so a replayed message dated only by the resend would
    // otherwise be created at the moment it was replayed, and the chain's
    // first `createdat` would move every time a session gapped. It is below a
    // stated `createdat` because that is the caller's own statement, and
    // above the snapshot because the snapshot is when the event happened
    // rather than when the message was made.
    let created = source(Role::Created)
        .or_else(|| stated_clock(122))
        .unwrap_or_else(|| event.clone());
    let defaults = [
        updated,
        created,
        Scalar::Null,
        Scalar::Null,
        Scalar::from(""),
        snapshot,
        sending,
    ];
    for role in Role::ALL {
        let at = if let Some(at) = roles[role as usize] {
            at
        } else {
            let (tag, name) = role.identity();
            let known = registry.get_field_by_tag(tag).ok_or_else(|| {
                refused(
                    name,
                    "a registered mandatory definition",
                    "missing definition",
                )
            })?;
            members.push(known.clone());
            values.push(Scalar::Null);
            let at = members.len() - 1;
            roles[role as usize] = Some(at);
            at
        };
        if members[at].as_fix().tag()? != Some(role.identity().0) {
            members[at].as_fix_mut().set_tag(role.identity().0)?;
        }
        // Every role but the snapshot is a value the message always has, so
        // its column cannot be null. `snapshotat` is the reading rather than
        // the message, and stays nullable.
        members[at].set_nullable(matches!(role, Role::Snapshot));
        if values[at].is_null() {
            values[at] = defaults[role as usize].clone();
        }
    }
    field = Field::new_with_metadata(
        field.name(),
        DataType::from_fields(members)?,
        field.is_nullable(),
        field.metadata.clone(),
    );
    let plan = super::schema::column_plan_of(&field, registry)?;
    let hard = plan
        .identity
        .finalize(&field, &mut values, Assertions::STATED)?;
    Ok((field, Scalar::from_sequence(values), hard))
}

fn now() -> Result<Scalar> {
    let nanos = crate::hashing::txhash::unix_now(TimeUnit::Nanosecond)?;
    Scalar::datetime64(nanos, TimeUnit::Nanosecond, Timezone::UTC)
}
