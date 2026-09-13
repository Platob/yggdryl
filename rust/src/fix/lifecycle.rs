//! Live named chains, arrival history and epoch-aligned snapshots.
//!
//! An explicit nonempty `code` names one chain globally. Otherwise the first
//! identifier reaching a live chain under the effective `instuuid` supplies
//! its code; a new chain is named `<scope UUID or ->/<first identifier>`.
//! Identifiers come from stated `altids` or the registered compiled selector,
//! never a second tag list. Occupied keys are not stolen, and empty code opens
//! no chain. The message's identity owner hashes the settled code into `puuid`.
//!
//! Every accepted message has `updatedat` truncated to its epoch bucket; its
//! real event instant remains `snapshotat`. One transition finalizes the whole
//! message before remembering its previous-message pair. `fill` returns every
//! result; `snapshot` emits only an off-grid arrival above its live chain's
//! highest consumed bucket. Suppression changes neither processing nor history.
//! Already-aligned arrivals consume a bucket without emitting a snapshot.
//!
//! State is bounded to live chains and their distinct attached identifiers:
//! code, last clock/UUID and highest bucket, never pending rows or historical
//! tombstones. A terminal message receives its previous pair, then closes the
//! chain even when suppressed. Reopening starts fresh and may emit in the same
//! bucket. Late arrivals advance history but cannot lower the high-water mark.
//! All stamps affect the semantic row alone, not wire entries or arrival digest.

use std::collections::{HashMap, hash_map::Entry};
use std::sync::Arc;

use smol_str::{SmolStr, format_smolstr};

use crate::hashing::xxhash::Xxh128;
use crate::path::{Path, Segment};
use crate::types::{Code, State, Uuid};
use crate::{DataType, Error, Result, Scalar, TimeUnit, Timezone};

use super::msg::FixMsg;
use super::registry::FixRegistry;
use super::schema::CLOCK_DATATYPE;
use super::{
    ALTIDS_TAG_NAME, CODE_TAG_NAME, FixKey, INSTUUID_TAG_NAME, ISINCODE_TAG_NAME, MICCODE_TAG_NAME,
    PREVTIMESTAMP_TAG_NAME, PREVUUID_TAG_NAME, PUUID_TAG_NAME, STATE_TAG_NAME, UPDATEDAT_TAG_NAME,
    UUID_TAG_NAME,
};

/// The separator between the parts an instrument's identity digests.
///
/// A unit separator rather than nothing, so `XNAS` beside `ESVUFR` and
/// `XNASE` beside `SVUFR` are two identities.
const PART_SEPARATOR: u8 = 0x1F;

/// The state a stream of messages has reached, one chain per live event.
///
/// Built once per stream and fed every message in order through
/// [`fill`](Self::fill); [`FixCodec::lifecycle`](super::FixCodec::lifecycle)
/// does exactly that at the default cadence. [`Self::snapshots`] owns a
/// configured lifecycle over a fallible message stream and drops only
/// successful messages whose buckets have already been consumed.
///
/// ```
/// use std::sync::Arc;
/// use yggdryl::{FixCodec, FixLifecycle, FixRegistry, Scalar, CODE_TAG_NAME, PREVUUID_TAG_NAME, PREVTIMESTAMP_TAG_NAME};
///
/// # fn main() -> yggdryl::Result<()> {
/// let registry = Arc::new(FixRegistry::new());
/// let reader = FixCodec::new(Arc::clone(&registry));
/// let mut life = FixLifecycle::new(registry);
///
/// let event = reader.parse_fix_line(
///     b"8=FIX.4.4|35=D|52=20260102-10:15:30.250|10=0|",
/// )?.with_value(CODE_TAG_NAME.0, Scalar::from("order/A1"))?;
/// let first = life.fill(event.clone())?;
/// let second = life.fill(event)?;
/// assert_eq!(first.puuid(), second.puuid());
/// assert!(first.by_tag(PREVUUID_TAG_NAME.0)?.is_null());
/// assert!(first.by_tag(PREVTIMESTAMP_TAG_NAME.0)?.is_null());
/// assert_eq!(second.by_tag(PREVUUID_TAG_NAME.0)?, first.uuid());
/// assert_eq!(second.by_tag(PREVTIMESTAMP_TAG_NAME.0)?, first.updatedat());
/// assert_eq!(life.alive(), 1);
/// life.clear();
/// assert_eq!(life.alive(), 0);
/// # Ok(())
/// # }
/// ```
#[derive(Debug)]
pub struct FixLifecycle {
    /// The owner of compiled message identifier selection.
    registry: Arc<FixRegistry>,
    /// Positive nanoseconds; changes require an empty live set.
    interval_ns: i64,
    /// Live chains only; capacity is reused after a wider live set closes.
    chains: HashMap<Uuid, Chain>,
    /// Which chain each live identifier belongs to.
    keys: HashMap<(Option<Uuid>, SmolStr), Uuid>,
}

/// One event still alive.
#[derive(Debug)]
struct Chain {
    /// Collision detection and identifier-only joins need the original name.
    code: SmolStr,
    /// Every attached scope is forgotten together when this chain closes.
    keys: Vec<(Option<Uuid>, SmolStr)>,
    /// Only the last successful message, under the shared clock datatype.
    timestamp: Scalar,
    uuid: Uuid,
    /// Includes aligned and suppressed arrivals, never decreases while live.
    highest_bucket: i64,
}

impl FixLifecycle {
    /// A second, in nanoseconds: a deterministic epoch grid, not a timer.
    pub const DEFAULT_INTERVAL_NS: i64 = 1_000_000_000;

    /// A stream with no event alive yet, using [`Self::DEFAULT_INTERVAL_NS`].
    #[must_use]
    pub fn new(registry: Arc<FixRegistry>) -> Self {
        Self {
            registry,
            interval_ns: Self::DEFAULT_INTERVAL_NS,
            chains: HashMap::new(),
            keys: HashMap::new(),
        }
    }

    /// The selected epoch-grid interval in nanoseconds.
    #[must_use]
    pub const fn interval_ns(&self) -> i64 {
        self.interval_ns
    }

    /// Selects a positive interval before any live chain uses it.
    ///
    /// Repeating the selected interval is a no-op, even while chains are live.
    ///
    /// # Errors
    ///
    /// Refuses a nonpositive interval or a change while a chain is live,
    /// without changing either configuration or stream state.
    pub fn set_interval_ns(&mut self, interval_ns: i64) -> Result<()> {
        if interval_ns <= 0 {
            return Err(Error::InvalidRecord {
                path: "$.interval_ns".into(),
                reason: crate::text::expected_got("positive nanoseconds", interval_ns),
            });
        }
        if interval_ns != self.interval_ns && !self.chains.is_empty() {
            return Err(Error::InvalidRecord {
                path: "$.interval_ns".into(),
                reason: crate::text::expected_got(
                    "no live chains before changing the interval",
                    format_args!("{} live chains", self.alive()),
                ),
            });
        }
        self.interval_ns = interval_ns;
        Ok(())
    }

    /// [`Self::set_interval_ns`], consuming the lifecycle.
    ///
    /// # Errors
    ///
    /// Returns the setter's refusal without changing stream state.
    pub fn try_with_interval_ns(mut self, interval_ns: i64) -> Result<Self> {
        self.set_interval_ns(interval_ns)?;
        Ok(self)
    }

    /// How many events are alive: opened by a message and not yet closed by
    /// a terminal state.
    #[must_use]
    pub fn alive(&self) -> usize {
        self.chains.len()
    }

    /// Forgets every chain and bucket, retaining the configured interval.
    pub fn clear(&mut self) {
        self.chains.clear();
        self.keys.clear();
    }

    /// Normalizes one message to its grid, then advances or closes its chain.
    ///
    /// A nonempty code selects its chain globally. Otherwise the first scoped
    /// identifier reaching a live chain wins; the first identifier names a new
    /// chain when none match. Occupied identifiers are never stolen.
    /// Each absent/null previous stamp comes from the selected chain's last
    /// message; each stated non-null stamp is preserved. A fresh or cleared
    /// replay rebuilds the same state from the carried identities. Feeding an
    /// earlier message into advanced state is a new arrival, not a rewind.
    /// Entries are untouched; all stamps land through one [`FixMsg::set_many`].
    ///
    /// # Errors
    ///
    /// Returns a located refusal for an unrepresentable bucket, malformed
    /// identifier/previous values, a code-hash collision, or a native stamp
    /// target refusal. No chain, history, bucket or identifier changes on error.
    pub fn fill(&mut self, message: FixMsg) -> Result<FixMsg> {
        self.transition(message).map(|(message, _)| message)
    }

    /// Processes one message, emitting only its live incarnation's new bucket.
    ///
    /// An already-aligned arrival consumes its bucket without emission. Equal
    /// and older buckets, and messages with no chain name, also return `None`.
    /// Suppressed messages still advance history and terminal messages close.
    ///
    /// # Errors
    ///
    /// Returns the same atomic refusal as [`Self::fill`].
    pub fn snapshot(&mut self, message: FixMsg) -> Result<Option<FixMsg>> {
        self.transition(message)
            .map(|(message, emitted)| emitted.then_some(message))
    }

    /// Owns this configured lifecycle over a fallible stream, without buffering.
    ///
    /// Only successful suppressed messages disappear. Source and transition
    /// errors remain items; the next input may succeed without the failed item
    /// having advanced state. Exhaustion is fused, even if the source is not.
    pub fn snapshots<I>(
        mut self,
        messages: I,
    ) -> impl std::iter::FusedIterator<Item = Result<FixMsg>> + use<I>
    where
        I: IntoIterator<Item = Result<FixMsg>>,
    {
        messages.into_iter().fuse().filter_map(move |message| {
            message
                .and_then(|message| self.snapshot(message))
                .transpose()
        })
    }

    /// Stage once, finalize once, then publish the live-state transition.
    fn transition(&mut self, mut message: FixMsg) -> Result<(FixMsg, bool)> {
        let incoming = message
            .updatedat()
            .as_datetime64()
            .ok_or_else(|| Error::InvalidRecord {
                path: Path::root().field(UPDATEDAT_TAG_NAME.1).render().into(),
                reason: crate::text::expected_got(
                    &CLOCK_DATATYPE,
                    crate::text::elide_display(&format_args!("{:?}", message.updatedat())),
                ),
            })?
            .0;
        let grid = i128::from(incoming).div_euclid(i128::from(self.interval_ns))
            * i128::from(self.interval_ns);
        let grid = i64::try_from(grid).map_err(|_| Error::InvalidRecord {
            path: Path::root().field(UPDATEDAT_TAG_NAME.1).render().into(),
            reason: crate::text::expected_got("an i64 nanosecond grid instant", grid),
        })?;
        let stated_instrument = stated_uuid(&message, INSTUUID_TAG_NAME)?;
        let instrument =
            stated_instrument.or_else(|| instrument_digest(&message).map(Uuid::from_v8));
        let keys = self.chain_keys(&message, instrument)?;
        let (code, persistent) = self.chain_name(&message, &keys)?;
        let previous = persistent.and_then(|held| self.chains.get(&held));
        let emitted = persistent.is_some()
            && incoming != grid
            && previous.is_none_or(|chain| grid > chain.highest_bucket);
        let terminal = is_terminal(&message);
        message.stamp_lifecycle(
            &code,
            Scalar::datetime64(grid, TimeUnit::Nanosecond, Timezone::UTC)?,
            instrument.filter(|_| stated_instrument.is_none()),
            previous,
        )?;
        if let Some(persistent) = persistent {
            if terminal {
                self.close(persistent);
            } else {
                let uuid = native_uuid(message.uuid(), UUID_TAG_NAME.1)?;
                self.join(
                    persistent,
                    code,
                    keys,
                    message.updatedat().clone(),
                    uuid,
                    grid,
                );
            }
        }
        Ok((message, emitted))
    }

    /// Explicit names precede scoped aliases; the identity owner hashes new names.
    fn chain_name(
        &self,
        message: &FixMsg,
        keys: &[(Option<Uuid>, SmolStr)],
    ) -> Result<(SmolStr, Option<Uuid>)> {
        let stated = message.get_by_tag(CODE_TAG_NAME.0);
        let Some(Scalar::String(stated)) = stated else {
            return Err(Error::InvalidRecord {
                path: Path::root().field(CODE_TAG_NAME.1).render().into(),
                reason: crate::text::expected_got(
                    "a non-null UTF-8 chain code",
                    crate::text::elide_display(&format_args!("{stated:?}")),
                ),
            });
        };
        let (code, persistent) = if !stated.as_str().is_empty() {
            (
                stated.storage().clone(),
                native_uuid(message.puuid(), PUUID_TAG_NAME.1)?,
            )
        } else if let Some(persistent) = keys.iter().find_map(|key| self.keys.get(key)) {
            let chain = &self.chains[persistent];
            return Ok((chain.code.clone(), Some(*persistent)));
        } else if let Some((scope, first)) = keys.first() {
            let code = match scope {
                Some(scope) => format_smolstr!("{scope}/{first}"),
                None => format_smolstr!("-/{first}"),
            };
            let persistent = super::identity::persistent_uuid(&code);
            (code, persistent)
        } else {
            return Ok((SmolStr::default(), None));
        };
        if let Some(chain) = self.chains.get(&persistent) {
            if chain.code != code {
                return Err(Error::InvalidRecord {
                    path: Path::root().field(PUUID_TAG_NAME.1).render().into(),
                    reason: crate::text::expected_got(
                        format_args!("the live code {}", crate::text::elide_display(&chain.code)),
                        crate::text::elide_display(&code),
                    ),
                });
            }
        }
        Ok((code, Some(persistent)))
    }

    /// Attach only unowned keys, retaining the incoming buffer for a new chain.
    fn join(
        &mut self,
        persistent: Uuid,
        code: SmolStr,
        mut keys: Vec<(Option<Uuid>, SmolStr)>,
        timestamp: Scalar,
        uuid: Uuid,
        bucket: i64,
    ) {
        keys.retain(|key| match self.keys.entry(key.clone()) {
            Entry::Vacant(entry) => {
                entry.insert(persistent);
                true
            }
            Entry::Occupied(_) => false,
        });
        match self.chains.entry(persistent) {
            Entry::Vacant(entry) => {
                // Duplicate occurrences cannot leave a large backing buffer
                // on a chain holding only a few distinct identifiers.
                keys.shrink_to_fit();
                entry.insert(Chain {
                    code,
                    keys,
                    timestamp,
                    uuid,
                    highest_bucket: bucket,
                });
            }
            Entry::Occupied(mut entry) => {
                let chain = entry.get_mut();
                chain.keys.extend(keys);
                chain.timestamp = timestamp;
                chain.uuid = uuid;
                chain.highest_bucket = chain.highest_bucket.max(bucket);
            }
        }
    }

    /// Forgets one chain and every identifier that reached it.
    fn close(&mut self, persistent: Uuid) {
        if let Some(chain) = self.chains.remove(&persistent) {
            for key in chain.keys {
                self.keys.remove(&key);
            }
        }
    }

    /// Resolve the stated map, or the same compiled projection enrichment uses.
    fn chain_keys(
        &self,
        message: &FixMsg,
        instrument: Option<Uuid>,
    ) -> Result<Vec<(Option<Uuid>, SmolStr)>> {
        let derived;
        let held = if let Some(held) = message
            .get_by_tag(ALTIDS_TAG_NAME.0)
            .filter(|held| !held.is_null())
        {
            held
        } else {
            let code = message
                .get_by_tag(35)
                .and_then(Scalar::as_str)
                .unwrap_or_default();
            let Some(component) = self.registry.get_msgtype(code) else {
                return Ok(Vec::new());
            };
            derived = component.identifier_mapping(message)?;
            &derived
        };
        let root = Path::root();
        let path = root.field(ALTIDS_TAG_NAME.1);
        let entries = held.as_mapping().ok_or_else(|| Error::InvalidRecord {
            path: path.render().into(),
            reason: crate::text::expected_got(
                "a sorted identifier Map",
                crate::text::elide_display(&format_args!("{held:?}")),
            ),
        })?;
        let mut keys: Vec<(Option<Uuid>, SmolStr)> = Vec::with_capacity(entries.len());
        let mut previous = None;
        for (index, (name, value)) in entries.iter().enumerate() {
            let Scalar::String(name) = name else {
                return Err(Error::InvalidRecord {
                    path: path.child(Segment::MapKey(index)).render().into(),
                    reason: crate::text::expected_got(
                        "a UTF-8 identifier name",
                        crate::text::elide_display(&format_args!("{name:?}")),
                    ),
                });
            };
            let name = name.as_str();
            if previous.is_some_and(|previous| previous >= name) {
                return Err(Error::InvalidRecord {
                    path: path.child(Segment::MapKey(index)).render().into(),
                    reason: crate::text::expected_got(
                        "unique identifier names in ascending order",
                        crate::text::elide_display(&name),
                    ),
                });
            }
            previous = Some(name);
            if value.is_null() {
                continue;
            }
            let Scalar::String(value) = value else {
                return Err(Error::InvalidRecord {
                    path: path.child(Segment::MapValue(index)).render().into(),
                    reason: crate::text::expected_got(
                        "a UTF-8 identifier or null",
                        crate::text::elide_display(&format_args!("{value:?}")),
                    ),
                });
            };
            // The index admits each key once when the join publishes; no
            // quadratic duplicate scan is needed while reading occurrences.
            if !value.as_str().is_empty() {
                keys.push((instrument, value.storage().clone()));
            }
        }
        Ok(keys)
    }
}

impl FixMsg {
    /// Preserve stated previous values and publish all native stamps together.
    fn stamp_lifecycle(
        &mut self,
        code: &SmolStr,
        updatedat: Scalar,
        instrument: Option<Uuid>,
        previous: Option<&Chain>,
    ) -> Result<()> {
        let previous_id_stated = stated_uuid(self, PREVUUID_TAG_NAME)?.is_some();
        let previous_clock_stated = stated_previous_clock(self)?.is_some();
        let previous = [
            (!previous_clock_stated).then(|| {
                (
                    PREVTIMESTAMP_TAG_NAME.0,
                    previous.map_or(Scalar::Null, |chain| chain.timestamp.clone()),
                )
            }),
            (!previous_id_stated).then(|| {
                (
                    PREVUUID_TAG_NAME.0,
                    previous.map_or(Scalar::Null, |chain| Scalar::Uuid(chain.uuid)),
                )
            }),
        ];
        self.set_many_with(
            [
                (CODE_TAG_NAME.0, Scalar::from(code.clone())),
                (UPDATEDAT_TAG_NAME.0, updatedat),
            ]
            .into_iter()
            .chain(instrument.map(|value| (INSTUUID_TAG_NAME.0, Scalar::Uuid(value))))
            .chain(previous.into_iter().flatten()),
            |key, field| {
                let expected = match key {
                    FixKey::Tag(tag) if *tag == CODE_TAG_NAME.0 => &DataType::utf8(),
                    FixKey::Tag(tag)
                        if *tag == UPDATEDAT_TAG_NAME.0 || *tag == PREVTIMESTAMP_TAG_NAME.0 =>
                    {
                        &CLOCK_DATATYPE
                    }
                    _ => &DataType::Uuid,
                };
                if field.dtype() == expected {
                    Ok(())
                } else {
                    Err(Error::InvalidRecord {
                        path: Path::root().field(field.name()).render().into(),
                        reason: crate::text::expected_got(
                            expected,
                            crate::text::elide_display(field.dtype()),
                        ),
                    })
                }
            },
        )
    }
}

/// Native UUIDs are already packed; this adds no version/variant policy.
fn stated_uuid(message: &FixMsg, (tag, name): (i32, &str)) -> Result<Option<Uuid>> {
    message
        .get_by_tag(tag)
        .filter(|held| !held.is_null())
        .map(|held| native_uuid(held, name))
        .transpose()
}

/// Extract the native leaf without coercion, parsing or changing its bits.
fn native_uuid(held: &Scalar, name: &str) -> Result<Uuid> {
    match held {
        Scalar::Uuid(held) => Ok(*held),
        held => Err(Error::InvalidRecord {
            path: Path::root().field(name).render().into(),
            reason: crate::text::expected_got(
                "a native UUID or null",
                crate::text::elide_display(&format_args!("{held:?}")),
            ),
        }),
    }
}

/// A previous clock is a statement in the declared layout, not a coercion.
fn stated_previous_clock(message: &FixMsg) -> Result<Option<&Scalar>> {
    let held = message
        .get_by_tag(PREVTIMESTAMP_TAG_NAME.0)
        .filter(|held| !held.is_null());
    match held {
        None => Ok(None),
        Some(held) if matches!(held.as_datetime64(), Some((_, TimeUnit::Nanosecond, zone)) if *zone == Timezone::UTC) => {
            Ok(Some(held))
        }
        Some(held) => Err(Error::InvalidRecord {
            path: Path::root().field(PREVTIMESTAMP_TAG_NAME.1).render().into(),
            reason: crate::text::expected_got(
                &CLOCK_DATATYPE,
                crate::text::elide_display(&format_args!("{held:?}")),
            ),
        }),
    }
}

/// The instrument's identity: the xxh128 digest of its market, its
/// classification, its ISIN - else its symbol - and its currency, or nothing
/// where the message names none of them.
fn instrument_digest(message: &FixMsg) -> Option<u128> {
    let first = |tags: &[i32]| -> Option<String> {
        tags.iter().find_map(|tag| {
            message
                .get_by_tag(*tag)
                .filter(|held| !held.is_null())
                .and_then(Scalar::as_str)
                .map(str::trim)
                .filter(|held| !held.is_empty())
                .map(str::to_ascii_uppercase)
        })
    };
    let market = first(&[MICCODE_TAG_NAME.0, 207, 100, 30]);
    let classification = first(&[461]);
    let isin = first(&[ISINCODE_TAG_NAME.0]).or_else(|| {
        (message.get_by_tag(22).and_then(Scalar::as_str) == Some("4"))
            .then(|| first(&[48]))
            .flatten()
    });
    let instrument = isin.or_else(|| first(&[55]));
    let currency = first(&[15]);
    if [&market, &classification, &instrument, &currency]
        .iter()
        .all(|part| part.is_none())
    {
        return None;
    }
    let mut digest = Xxh128::new();
    for part in [market, classification, instrument, currency] {
        if let Some(held) = part {
            digest.write_bytes(held.as_bytes());
        }
        digest.write_bytes(&[PART_SEPARATOR]);
    }
    Some(digest.as_u128())
}

/// Whether the state the message reports ends the order's life.
///
/// The crate's own `state`, else `OrdStatus`, else `ExecType`, read as the
/// one lifecycle vocabulary; a message stating none is not an ending.
fn is_terminal(message: &FixMsg) -> bool {
    [STATE_TAG_NAME.0, 39, 150]
        .into_iter()
        .filter_map(|tag| message.get_by_tag(tag))
        .find(|held| !held.is_null())
        .and_then(|held| match held {
            Scalar::Code(Code::State(state)) => Some(state.clone()),
            other => other.as_str().and_then(State::from_spelling),
        })
        .is_some_and(|state| !state.is_live())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stated_identifier_keys_share_long_strings() {
        let registry = Arc::new(FixRegistry::new());
        let field = registry
            .get_group_by_counter(ALTIDS_TAG_NAME.0)
            .unwrap()
            .clone();
        let identifier = Scalar::from("identifier".repeat(128));
        let message = FixMsg::with_registry(
            Arc::clone(&registry),
            DataType::from_fields([field])
                .unwrap()
                .required_field("event"),
            Scalar::from_sequence([Scalar::from_mapping([(
                Scalar::from("id"),
                identifier.clone(),
            )])
            .unwrap()]),
        )
        .unwrap();
        let life = FixLifecycle::new(registry);
        let keys = life.chain_keys(&message, None).unwrap();
        assert_eq!(keys.len(), 1);
        assert_eq!(
            keys[0].1.as_str().as_ptr(),
            identifier.as_str().unwrap().as_ptr()
        );
    }

    #[test]
    fn duplicate_occurrences_do_not_enlarge_a_live_chains_key_buffer() {
        for count in [1, 64, 1024] {
            let mut life = FixLifecycle::new(Arc::new(FixRegistry::new()));
            let persistent = Uuid::new(1);
            life.join(
                persistent,
                SmolStr::new("same"),
                vec![(None, SmolStr::new("same")); count],
                Scalar::datetime64(0, TimeUnit::Nanosecond, Timezone::UTC).unwrap(),
                Uuid::new(2),
                0,
            );
            let chain = &life.chains[&persistent];
            assert_eq!(chain.keys.len(), 1);
            assert_eq!(chain.keys.capacity(), 1);
            assert_eq!(life.keys.len(), 1);
            life.close(persistent);
            assert!(life.chains.is_empty());
            assert!(life.keys.is_empty());
        }
    }

    #[test]
    fn code_hash_collision_cannot_advance_close_or_attach_to_another_name() {
        let registry = Arc::new(FixRegistry::new());
        let codec = super::super::FixCodec::new(Arc::clone(&registry));
        for explicit in [false, true] {
            let code = if explicit { "A" } else { "-/A" };
            let mut message = codec
                .parse_fix_line(b"8=FIX.4.4|35=D|52=20260102-10:15:30.125|10=0|")
                .unwrap();
            message
                .set_many([
                    (
                        CODE_TAG_NAME.0,
                        Scalar::from(if explicit { code } else { "" }),
                    ),
                    (
                        ALTIDS_TAG_NAME.0,
                        Scalar::from_mapping([(Scalar::from("id"), Scalar::from("A"))]).unwrap(),
                    ),
                    (STATE_TAG_NAME.0, Scalar::from("Filled")),
                ])
                .unwrap();
            let persistent = super::super::identity::persistent_uuid(code);
            let mut life = FixLifecycle::new(Arc::clone(&registry));
            // Force the otherwise impractical 122-bit collision at the state
            // seam, without weakening the public asserted-identity boundary.
            life.join(
                persistent,
                SmolStr::new("a different live code"),
                vec![(None, SmolStr::new("OWNED"))],
                Scalar::datetime64(0, TimeUnit::Nanosecond, Timezone::UTC).unwrap(),
                Uuid::new(7),
                0,
            );
            let Error::InvalidRecord { path, .. } = life.snapshot(message).unwrap_err() else {
                panic!("a located code collision");
            };
            assert_eq!(path, "$.puuid");
            let chain = &life.chains[&persistent];
            assert_eq!(chain.code, "a different live code");
            assert_eq!(chain.uuid, Uuid::new(7));
            assert_eq!(chain.highest_bucket, 0);
            assert_eq!(chain.keys, [(None, SmolStr::new("OWNED"))]);
            assert_eq!(life.keys.len(), 1);
            assert!(!life.keys.contains_key(&(None, SmolStr::new("A"))));
            assert_eq!(life.alive(), 1);
        }
    }
}
