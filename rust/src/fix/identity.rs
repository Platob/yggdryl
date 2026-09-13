//! The one boundary settling clocks and named-content identity.

use std::sync::Arc;

use crate::hashing::{txhash::TxHash, xxhash};
use crate::types::Uuid;
use crate::{DataType, Digest, Error, Field, Result, Scalar, TimeUnit, Timezone};

use super::schema::{CLOCK_DATATYPE, ENTRIES_COLUMN};
use super::{
    CODE_TAG_NAME, CREATEDAT_TAG_NAME, FixRegistry, PUUID_TAG_NAME, SNAPSHOTAT_TAG_NAME,
    UPDATEDAT_TAG_NAME, UUID_TAG_NAME,
};

/// The replay bundle, in one fixed internal order unrelated to column order.
#[derive(Clone, Copy)]
pub(super) enum Role {
    Updated,
    Created,
    Uuid,
    Persistent,
    Code,
    Snapshot,
    Sending,
}

impl Role {
    pub(super) const ALL: [Self; 7] = [
        Self::Updated,
        Self::Created,
        Self::Uuid,
        Self::Persistent,
        Self::Code,
        Self::Snapshot,
        Self::Sending,
    ];

    pub(super) const fn identity(self) -> (i32, &'static str) {
        match self {
            Self::Updated => UPDATEDAT_TAG_NAME,
            Self::Created => CREATEDAT_TAG_NAME,
            Self::Uuid => UUID_TAG_NAME,
            Self::Persistent => PUUID_TAG_NAME,
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
            Self::Uuid | Self::Persistent => DataType::Uuid,
            Self::Code => DataType::utf8(),
            _ => CLOCK_DATATYPE,
        }
    }
}

pub(super) fn is_mandatory(tag: i32) -> bool {
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
        if tag != claim && is_mandatory(claim) {
            return Err(refused(
                field.name(),
                format_args!("FIX tag {claim} claimed by its name"),
                format_args!("explicit tag {tag}"),
            ));
        }
    }
    Ok(explicit.or(named))
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
            if field.name() != ENTRIES_COLUMN
                && !tag.is_some_and(|tag| {
                    [UUID_TAG_NAME.0, UPDATEDAT_TAG_NAME.0, CREATEDAT_TAG_NAME.0].contains(&tag)
                })
            {
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
            let at = self.index(role)?;
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
            if matches!(role, Role::Uuid | Role::Persistent) && values[at].is_null() {
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
        let persistent = Scalar::Uuid(persistent_uuid(code));
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
        let uuid = Scalar::Uuid(
            TxHash::new_in(
                nanos,
                TimeUnit::Nanosecond,
                Digest::new(crate::DigestAlgorithm::Xxh64, u128::from(state.as_u64())),
            )?
            .into_uuid()?,
        );
        let uuid_at = self.index(Role::Uuid)?;
        assert_identity(
            &schema.fields()[uuid_at],
            &values[uuid_at],
            &uuid,
            assertions.uuid,
        )?;
        values[uuid_at] = uuid;
        Ok(Hard {
            updatedat: values[updated_at].clone(),
            createdat: values[self.index(Role::Created)?].clone(),
            uuid: values[uuid_at].clone(),
            puuid: values[persistent_at].clone(),
        })
    }
}

#[derive(Clone, Copy, Default)]
pub(super) struct Assertions {
    pub(super) uuid: bool,
    pub(super) persistent: bool,
}

impl Assertions {
    pub(super) const STATED: Self = Self {
        uuid: true,
        persistent: true,
    };
}

/// Moved into the message's four direct fields after the row is proven.
pub(super) struct Hard {
    pub(super) updatedat: Scalar,
    pub(super) createdat: Scalar,
    pub(super) uuid: Scalar,
    pub(super) puuid: Scalar,
}

pub(super) fn persistent_uuid(code: &str) -> Uuid {
    Uuid::from_v8(xxhash::xxh128(code.as_bytes()))
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
        DataType::Uuid => matches!(value, Scalar::Uuid(_)),
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
    let transact = initial
        .iter()
        .position(|column| column.tag == Some(60))
        .and_then(|at| values.get(at))
        .filter(|value| !value.is_null())
        .cloned();
    let snapshot = source(Role::Snapshot)
        .or(transact)
        .unwrap_or_else(|| sending.clone());
    let updated = source(Role::Updated).unwrap_or_else(|| snapshot.clone());
    let created = source(Role::Created).unwrap_or_else(|| snapshot.clone());
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
        members[at].set_nullable(false);
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
