//! The market extension point: one closed shape over registered enum kinds.
//!
//! A registered kind - `side`, `timeinforce`, a crate's own vocabulary - is
//! a [`MarketDescriptor`], one `static` in the kind's own root file, claimed
//! once through [`claim`] under the byte, the name and the Arrow extension
//! name it states. The four root enums hold one variant for every kind:
//! [`DataType::Market`] carries a [`MarketType`], [`Field::Market`] a field
//! of one, [`Scalar::Market`] a [`MarketScalar`] and
//! [`Serie::Market`](crate::Serie::Market) a [`MarketSerie`] in one of the
//! two storages every kind shares, Arrow's `UInt8` or `UInt16` holding the
//! members' codes. Nothing in the core names a kind: a parsed name, a serde
//! tag, a value-stream byte and an Arrow extension name each reach their kind
//! through the register, and a name no claim answers is refused naming the
//! registration it lacks. The registered codes - `isin`, `ccy` and the
//! fifteen beside them - are not kinds: a code is the core's own, a flat
//! variant of every root enum, because an identifier is generic where a
//! market vocabulary is not.
//!
//! What a kind states beside its storage are the three numbers its values
//! and its datatype order and hash by (`value_rank`, `dtype_rank`, `shape`),
//! wire contracts the core's kinds carried before they were registered and
//! never move; a kind claimed later takes the reserved numbers.

use std::cmp::Ordering;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex, OnceLock};

use arrow_schema::DataType as ArrowDataType;
use smol_str::{SmolStr, format_smolstr};

use crate::plugin::Register;
use crate::serie::{UInt8Serie, UInt16Serie};
use crate::{DataType, DataTypeId, DataTypeKind, Error, Field, Result, Scalar};

/// The Arrow storage a registered kind's column holds: its members' codes
/// at the width the widest code needs.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum MarketStorage {
    /// Codes that fit a byte, in Arrow's `UInt8`.
    Code8,
    /// Codes that pass 255, in Arrow's `UInt16`.
    Code16,
}

impl MarketStorage {
    /// The Arrow datatype a column of this storage is laid out in.
    #[must_use]
    pub const fn arrow(self) -> ArrowDataType {
        match self {
            Self::Code8 => ArrowDataType::UInt8,
            Self::Code16 => ArrowDataType::UInt16,
        }
    }

    /// Whether `code` is stored at this width.
    #[must_use]
    pub const fn fits(self, code: u16) -> bool {
        match self {
            Self::Code8 => code <= u8::MAX as u16,
            Self::Code16 => true,
        }
    }
}

/// One member of a registered kind: its stored code, its stored name and
/// what it means.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct MarketMember {
    /// The code a column stores for this member, at the kind's width.
    pub code: u16,
    /// The stored name, which every text format writes.
    pub name: &'static str,
    /// What this member means, in a sentence.
    pub description: &'static str,
}

/// What one registered kind states about itself, once, as a `static` in
/// its own root file.
///
/// The numbers are wire contracts: the byte is what the value stream writes
/// as a value's tag, `value_rank` orders and hashes a value against every
/// other kind of value, `dtype_rank` orders the datatype, `shape` hashes it.
/// The function pointer is the kind's own door: `read` is its spelling
/// reader, every spelling the kind accepts into a member's code.
pub struct MarketDescriptor {
    /// The identifier byte, in the Enum family's range.
    pub id: DataTypeId,
    /// The canonical lowercase name: the datatype's spelling, the serde tag
    /// and the parser's word.
    pub name: &'static str,
    /// The Arrow extension name a column rides under.
    pub extension_name: &'static str,
    /// The storage a column holds.
    pub storage: MarketStorage,
    /// The members in code order, at least one.
    pub members: &'static [MarketMember],
    /// The rank a value of this kind orders and hashes under among every
    /// kind of value.
    pub value_rank: u8,
    /// The rank the datatype orders under among every datatype.
    pub dtype_rank: u8,
    /// The position the datatype hashes as.
    pub shape: isize,
    /// A spelling into a member's code: the stored name, the standard's
    /// name folded, the wire value - whatever the kind reads - refused
    /// naming the kind where none matches.
    pub read: fn(&str) -> Result<u16>,
}

impl MarketDescriptor {
    /// The reserved value rank a kind claimed by another crate takes.
    pub const RESERVED_ENUM_VALUE_RANK: u8 = 33;
    /// The reserved datatype rank a kind claimed by another crate takes.
    pub const RESERVED_DTYPE_RANK: u8 = 81;
    /// The reserved shape position a kind claimed by another crate takes.
    pub const RESERVED_SHAPE: isize = 73;

    /// The datatype of this kind.
    #[must_use]
    pub const fn dtype(&'static self) -> DataType {
        DataType::Market(MarketType::new(self))
    }

    /// A field of this kind.
    #[must_use]
    pub fn field(&'static self, name: impl Into<SmolStr>, nullable: bool) -> Field {
        Field::new(name, self.dtype(), nullable)
    }

    /// The member one stored code names, or `None` where none does.
    #[must_use]
    pub fn member_of(&self, code: u16) -> Option<&'static MarketMember> {
        let members: &'static [MarketMember] = self.members;
        members
            .binary_search_by_key(&code, |member| member.code)
            .ok()
            .map(|index| &members[index])
    }

    /// The canonical default of this kind, as the value it is: the member
    /// at code zero, the one every kind stores for a side, a kind or a
    /// time in force nobody stated.
    ///
    /// # Errors
    ///
    /// Returns an error naming the kind where no member holds code zero.
    pub fn default_scalar(&'static self) -> Result<Scalar> {
        self.claimed()?;
        self.adopt_member(0)
    }

    /// This descriptor is the kind claimed under its byte, or the refusal
    /// naming the registration it lacks: the public doors that mint a value
    /// ask it once, so no descriptor never claimed - or refused at its
    /// claim - puts a value into a column, a stream or an order.
    fn claimed(&'static self) -> Result<()> {
        if kind_of(self.id).is_some_and(|claimed| std::ptr::eq(claimed, self)) {
            Ok(())
        } else {
            Err(unregistered(format_args!("{}", self.name)))
        }
    }

    /// The member one spelling names, through this kind's own reader, as
    /// the scalar it is.
    ///
    /// What the reader answers is held to the member table, so a kind whose
    /// reader disagrees with its descriptor refuses here, and every value
    /// in hand is one its column can hold.
    ///
    /// # Errors
    ///
    /// The kind's own refusal of the spelling, or the refusal of a code no
    /// member holds.
    pub fn scalar(&'static self, text: &str) -> Result<Scalar> {
        self.claimed()?;
        self.adopt_spelling(text)
    }

    /// [`Self::scalar`] for a kind a validated field already proved claimed:
    /// what the cast's text ingest takes per cell, so a cell pays the
    /// reader and the member search alone.
    pub(crate) fn adopt_spelling(&'static self, text: &str) -> Result<Scalar> {
        let code = (self.read)(text)?;
        if self.member_of(code).is_none() {
            return Err(Error::InvalidDataType {
                kind: self.name,
                reason: format_smolstr!(
                    "expected the reader of {} to answer a member's code, got {code}, which no member holds",
                    self.name
                ),
            });
        }
        Ok(Scalar::Market(MarketScalar::new(self, code)))
    }

    /// The member one stored code names, as the scalar it is.
    ///
    /// # Errors
    ///
    /// Returns an error naming the code where no member holds it, or
    /// naming the registration this kind lacks.
    pub fn member(&'static self, code: i64) -> Result<Scalar> {
        self.claimed()?;
        self.adopt_member(code)
    }

    /// [`Self::member`] for a kind a validated field already proved
    /// claimed: what the landing's cell read and the cast's per-value
    /// ingest take, so a row pays the member search alone.
    pub(crate) fn adopt_member(&'static self, code: i64) -> Result<Scalar> {
        u16::try_from(code)
            .ok()
            .and_then(|code| self.member_of(code))
            .map(|member| Scalar::Market(MarketScalar::new(self, member.code)))
            .ok_or_else(|| Error::InvalidDataType {
                kind: self.name,
                reason: format_smolstr!("expected the code of a {}, got {code}", self.name),
            })
    }

    /// A member's own code adopted as the value it is, unread: the door a
    /// typed member crosses into a scalar, which only the kind's own macro
    /// reaches; a code nothing proved goes through [`Self::member`].
    #[must_use]
    pub(crate) fn adopt_code(&'static self, code: u16) -> Scalar {
        Scalar::Market(MarketScalar::new(self, code))
    }
}

impl PartialEq for MarketDescriptor {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

impl Eq for MarketDescriptor {}

impl fmt::Debug for MarketDescriptor {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MarketDescriptor")
            .field("id", &self.id)
            .field("name", &self.name)
            .field("storage", &self.storage)
            .finish_non_exhaustive()
    }
}

impl fmt::Display for MarketDescriptor {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.name)
    }
}

/// The datatype payload of a registered kind: what [`DataType::Market`]
/// holds and a market field's datatype value.
///
/// Equal by the kind's byte and ordered by its datatype rank then the byte,
/// the two readings the kinds had as variants of their own. Its readers
/// borrow, so `kind.kind()` on a borrowed type reaches the descriptor
/// wherever [`DataTypeValue`](crate::DataTypeValue), whose `kind` is the
/// family, is in scope too.
#[derive(Clone, Copy)]
pub struct MarketType(&'static MarketDescriptor);

impl MarketType {
    /// The datatype of `kind`.
    #[must_use]
    pub const fn new(kind: &'static MarketDescriptor) -> Self {
        Self(kind)
    }

    /// The kind.
    #[must_use]
    pub const fn kind(&self) -> &'static MarketDescriptor {
        self.0
    }

    /// The kind's identifier.
    #[must_use]
    pub const fn id(&self) -> DataTypeId {
        self.0.id
    }

    /// The kind's name.
    #[must_use]
    pub const fn name(&self) -> &'static str {
        self.0.name
    }

    /// The storage a column of this kind holds.
    #[must_use]
    pub const fn storage(&self) -> MarketStorage {
        self.0.storage
    }
}

impl PartialEq for MarketType {
    fn eq(&self, other: &Self) -> bool {
        self.0.id == other.0.id
    }
}

impl Eq for MarketType {}

impl PartialOrd for MarketType {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for MarketType {
    fn cmp(&self, other: &Self) -> Ordering {
        self.0
            .dtype_rank
            .cmp(&other.0.dtype_rank)
            .then_with(|| self.0.id.cmp(&other.0.id))
    }
}

/// Nothing: a kind's marker hashes as every parameter-free marker does, so
/// a field of a kind hashes its name, its nullability and its metadata
/// alone - what the FIX dictionary's pinned hash reads - and the kind's
/// place is written by [`DataType`]'s own hash, as the position of the
/// variant a kind once was.
impl Hash for MarketType {
    fn hash<H: Hasher>(&self, _: &mut H) {}
}

impl fmt::Debug for MarketType {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.0.name)
    }
}

impl fmt::Display for MarketType {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.0.name)
    }
}

impl crate::DataTypeValue for MarketType {
    const FAMILY: &'static str = "market";

    type Sidecar = ();

    fn id(&self) -> DataTypeId {
        self.0.id
    }

    /// The kind is the one claimed under its byte - this very descriptor,
    /// not another stating the byte - so a descriptor never claimed, or
    /// refused at its claim, validates as no kind.
    fn validate(&self) -> Result<()> {
        if kind_of(self.0.id).is_some_and(|claimed| std::ptr::eq(claimed, self.0)) {
            Ok(())
        } else {
            Err(unregistered(format_args!("{}", self.0.name)))
        }
    }

    fn into_dtype(self) -> DataType {
        DataType::Market(self)
    }

    fn from_dtype(dtype: &DataType) -> Option<Self> {
        match dtype {
            DataType::Market(kind) => Some(*kind),
            _ => None,
        }
    }
}

/// One value of a registered kind: the kind and the member's code, built
/// only through the kind's own doors - its reader, its member table, its
/// typed members - so no crate lays out a code no member holds.
#[derive(Clone, Copy)]
pub struct MarketScalar {
    kind: &'static MarketDescriptor,
    code: u16,
}

impl MarketScalar {
    pub(crate) const fn new(kind: &'static MarketDescriptor, code: u16) -> Self {
        Self { kind, code }
    }

    /// The kind.
    #[must_use]
    pub const fn kind(&self) -> &'static MarketDescriptor {
        self.kind
    }

    /// The kind's identifier.
    #[must_use]
    pub const fn id(&self) -> DataTypeId {
        self.kind.id
    }

    /// The member's stored code.
    #[must_use]
    pub const fn code(&self) -> u16 {
        self.code
    }

    /// The member this value is; every value holds one, since a value
    /// enters only through its kind's reader or its member table.
    #[must_use]
    pub fn member(&self) -> Option<&'static MarketMember> {
        self.kind.member_of(self.code)
    }

    /// The member's stored name, what every text format writes.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        self.member().map_or("", |member| member.name)
    }

    /// The datatype of this value.
    #[must_use]
    pub const fn dtype(&self) -> DataType {
        self.kind.dtype()
    }

    /// The bytes every stored digest reads: the code at the kind's width,
    /// exactly what the kinds fed as variants of their own.
    pub(crate) fn hash_feed<H: Hasher>(&self, state: &mut H) {
        match self.kind.storage {
            MarketStorage::Code16 => state.write_u16(self.code),
            MarketStorage::Code8 => state.write_u8(u8::try_from(self.code).unwrap_or(u8::MAX)),
        }
    }

    /// The order among values of one rank: the kind's byte, then the code.
    pub(crate) fn order(&self, other: &Self) -> Ordering {
        self.kind
            .id
            .cmp(&other.kind.id)
            .then_with(|| self.code.cmp(&other.code))
    }
}

impl PartialEq for MarketScalar {
    fn eq(&self, other: &Self) -> bool {
        self.order(other) == Ordering::Equal
    }
}

impl Eq for MarketScalar {}

impl PartialOrd for MarketScalar {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for MarketScalar {
    fn cmp(&self, other: &Self) -> Ordering {
        self.kind
            .value_rank
            .cmp(&other.kind.value_rank)
            .then_with(|| self.order(other))
    }
}

impl Hash for MarketScalar {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.hash_feed(state);
    }
}

impl fmt::Debug for MarketScalar {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}({:?})", self.kind.name, self.as_str())
    }
}

impl fmt::Display for MarketScalar {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// A column of a registered kind, in one of the two storages every kind
/// shares; the leaf's field holds the [`MarketType`], so the kind is read
/// off the field and stored nowhere twice.
#[derive(Clone, Debug)]
pub enum MarketSerie {
    /// The members' codes as `uint8`.
    Code8(Arc<UInt8Serie>),
    /// The members' codes as `uint16`.
    Code16(Arc<UInt16Serie>),
}

/// What a typed member of a registered kind answers: the kind it is, and
/// the two conversions with the one [`Scalar`] - owned, because a member
/// is `Copy` and a scalar lends nothing.
pub trait MarketValue:
    Sized + Clone + fmt::Debug + fmt::Display + Eq + Ord + Hash + Send + Sync + 'static
{
    /// The kind this value is one of.
    const KIND: &'static MarketDescriptor;

    /// Widen this value to the one scalar.
    fn into_scalar(self) -> Scalar;

    /// Narrow a scalar to this value, `None` for a scalar of any other kind.
    fn from_scalar(value: &Scalar) -> Option<Self>;

    /// The datatype of this kind.
    #[must_use]
    fn dtype() -> DataType {
        Self::KIND.dtype()
    }
}

// ------------------------------------------------------------------------
// The register: one claim per byte, per name and per extension name, read
// by every intake door and by nothing that already holds a kind.
// ------------------------------------------------------------------------

/// What a crate names itself as when it claims its kinds.
const CORE: &str = "yggdryl";

static BY_NAME: Register<&'static str, &'static MarketDescriptor> = Register::new("market kind");
static BY_EXTENSION: Register<&'static str, &'static MarketDescriptor> =
    Register::new("market kind extension name");
static BY_BYTE: [OnceLock<&'static MarketDescriptor>; 256] = [const { OnceLock::new() }; 256];
static SEEDED: OnceLock<()> = OnceLock::new();
static CLAIMING: Mutex<()> = Mutex::new(());

/// The bytes no kind may claim again: the one a retired kind held.
const RETIRED: [u8; 1] = [0xc6];

/// The core's own kinds, in byte order: what the register seeds itself
/// with before it answers anything, until the crate that owns them claims
/// them through [`claim`] itself.
const CORE_KINDS: [&MarketDescriptor; 4] = [
    &crate::marketdatakind::MARKETDATAKIND_KIND,
    &crate::side::SIDE_KIND,
    &crate::marketdatatype::MARKETDATATYPE_KIND,
    &crate::timeinforce::TIMEINFORCE_KIND,
];

/// The core's own kinds, claimed before the register answers anything;
/// what every reader runs first, and what a claim of the logical names runs
/// before it takes the registers' lock, since the seeding takes it too.
pub(crate) fn seed() {
    SEEDED.get_or_init(|| {
        for kind in CORE_KINDS {
            // The core's claims cannot conflict: each byte, name and
            // extension name is stated once in the crate.
            claim_unseeded(kind, CORE).expect("the core's own kinds claim cleanly");
        }
    });
}

/// Claim `kind` for the crate `by`: its byte, its name and its extension
/// name, each once for the life of the process.
///
/// # Errors
///
/// Returns [`Error::Conflict`] naming the first claimant where the byte,
/// the name or the extension name is claimed already, and
/// [`Error::InvalidDataType`] where the kind disagrees with itself or with
/// the core: a claim in the core's own name, which only the core's seeding
/// makes; a byte outside the Enum family, the family's own number, the
/// retired byte, one the core's `state` holds; a claim stating anything
/// but the reserved `(value_rank, dtype_rank, shape)`; no member, members
/// out of code order or one past the storage's width; a name not in its
/// folded spelling or one the datatype grammar already reads; an extension
/// name the core recognizes.
pub fn claim(kind: &'static MarketDescriptor, by: &'static str) -> Result<()> {
    seed();
    if by == CORE {
        return Err(Error::InvalidDataType {
            kind: kind.name,
            reason: format_smolstr!(
                "expected the claiming crate's own name, got {CORE:?}, which the core's kinds alone claim as"
            ),
        });
    }
    claim_unseeded(kind, by)
}

/// The one lock a claim of any register takes: the market kinds' three
/// keys and the logical names are checked and taken under it, so a name
/// claimed on one is never answered by the other.
pub(crate) fn claiming() -> std::sync::MutexGuard<'static, ()> {
    CLAIMING
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn claim_unseeded(kind: &'static MarketDescriptor, by: &'static str) -> Result<()> {
    let byte = kind.id.as_u8();
    let family = kind.id.kind();
    let refuse = |reason: SmolStr| Error::InvalidDataType {
        kind: kind.name,
        reason,
    };
    if family != DataTypeKind::Enum {
        return Err(refuse(format_smolstr!(
            "expected a byte in the enum range {:#04x}..={:#04x}, got {byte:#04x} in the {family} family",
            DataTypeKind::Enum.id(),
            DataTypeKind::Enum.last(),
        )));
    }
    if byte == family.id() {
        return Err(refuse(format_smolstr!(
            "expected a leaf byte, got the {family} family's own number {byte:#04x}"
        )));
    }
    if RETIRED.contains(&byte) {
        return Err(refuse(format_smolstr!(
            "expected an unretired byte, got {byte:#04x}, which a retired kind held and no kind takes again"
        )));
    }
    if let Some(core) = kind.id.core_name() {
        return Err(refuse(format_smolstr!(
            "expected a free byte, got {byte:#04x}, which the core's `{core}` holds"
        )));
    }
    // The numbers a kind orders and hashes by are wire contracts the core's
    // own kinds carried as variants; any other claim takes the reserved
    // ones, so no rank or shape ever collides with the core's and nothing
    // sorts or hashes two kinds as one.
    if by != CORE {
        let expected = (
            MarketDescriptor::RESERVED_ENUM_VALUE_RANK,
            MarketDescriptor::RESERVED_DTYPE_RANK,
            MarketDescriptor::RESERVED_SHAPE,
        );
        if (kind.value_rank, kind.dtype_rank, kind.shape) != expected {
            return Err(refuse(format_smolstr!(
                "expected the reserved (value_rank, dtype_rank, shape) {expected:?}, got {:?}",
                (kind.value_rank, kind.dtype_rank, kind.shape)
            )));
        }
    }
    // The member table is what the codes are read by, in code order and at
    // the storage's width.
    if kind.members.is_empty() {
        return Err(refuse(
            "expected at least one member of an enum kind, got none".into(),
        ));
    }
    if let Some(pair) = kind
        .members
        .windows(2)
        .find(|pair| pair[0].code >= pair[1].code)
    {
        return Err(refuse(format_smolstr!(
            "expected members in ascending code order, got {} ({}) before {} ({})",
            pair[0].name,
            pair[0].code,
            pair[1].name,
            pair[1].code
        )));
    }
    if let Some(wide) = kind
        .members
        .iter()
        .find(|member| !kind.storage.fits(member.code))
    {
        return Err(refuse(format_smolstr!(
            "expected every member's code to fit {:?}, got {} ({})",
            kind.storage,
            wide.name,
            wide.code
        )));
    }
    // The name is read folded by every door, so it is claimed folded; one
    // the grammar already reads as a core datatype or a logical name would
    // never reach the register.
    if !kind
        .name
        .starts_with(|first: char| first.is_ascii_lowercase())
        || !kind
            .name
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
    {
        return Err(refuse(format_smolstr!(
            "expected a name in its folded spelling - a lowercase letter, then lowercase letters and digits - got {:?}",
            kind.name
        )));
    }
    if DataTypeId::ALL
        .iter()
        .any(|id| id.core_arrow_extension_name() == Some(kind.extension_name))
    {
        return Err(refuse(format_smolstr!(
            "expected an extension name of the kind's own, got {:?}, which the core recognizes",
            kind.extension_name
        )));
    }
    // One claim at a time: the three keys are checked together and taken
    // together, so no kind is ever half claimed and a refusal names the
    // first claimant of whichever key is held.
    let _claiming = claiming();
    if let Some(held) = BY_BYTE[usize::from(byte)].get() {
        return Err(Error::Conflict {
            expected: "market kind byte",
            actual: BY_NAME.claimant(held.name).unwrap_or(CORE),
            path: format_smolstr!("{byte:#04x} ({})", held.name),
        });
    }
    if let Some(first) = BY_NAME.claimant(kind.name) {
        return Err(Error::Conflict {
            expected: "market kind",
            actual: first,
            path: SmolStr::new(kind.name),
        });
    }
    // The grammar reads the register, which is seeding while the core's
    // own kinds claim, so the core's names - no word of the grammar, which
    // their tests pin - are not asked of it here.
    if by != CORE && DataType::from_str(kind.name).is_ok() {
        return Err(refuse(format_smolstr!(
            "expected a name the datatype grammar does not read, got {:?}",
            kind.name
        )));
    }
    if let Some(first) = BY_EXTENSION.claimant(kind.extension_name) {
        return Err(Error::Conflict {
            expected: "market kind extension name",
            actual: first,
            path: SmolStr::new(kind.extension_name),
        });
    }
    // The byte table is written first, so a name or an extension name
    // answered by a reader is a kind whose byte validates already; every
    // conflict was checked under the lock above.
    let _ = BY_BYTE[usize::from(byte)].set(kind);
    BY_NAME.claim(kind.name, kind, by)?;
    BY_EXTENSION.claim(kind.extension_name, kind, by)?;
    Ok(())
}

/// The kind one identifier byte is claimed by, or `None`.
#[must_use]
pub fn kind_of(id: DataTypeId) -> Option<&'static MarketDescriptor> {
    seed();
    BY_BYTE[usize::from(id.as_u8())].get().copied()
}

/// The kind one canonical name is claimed by, or `None`.
#[must_use]
pub fn kind_named(name: &str) -> Option<&'static MarketDescriptor> {
    seed();
    BY_NAME.get(name)
}

/// The kind one Arrow extension name is claimed by, or `None`.
#[must_use]
pub fn kind_for_extension(name: &str) -> Option<&'static MarketDescriptor> {
    seed();
    BY_EXTENSION.get(name)
}

/// Every claimed kind, in byte order.
#[must_use]
pub fn kinds() -> Vec<&'static MarketDescriptor> {
    seed();
    BY_BYTE
        .iter()
        .filter_map(|slot| slot.get().copied())
        .collect()
}

/// The refusal of a name, a byte or a tag no claim answers: `what` names
/// it - the word parsed, the serde tag, the value-stream byte - and the
/// refusal says to install the crate that claims it.
pub(crate) fn unregistered(what: fmt::Arguments<'_>) -> Error {
    Error::UnknownDataType(format_smolstr!("{what}"))
}
