//! One order's life, read across the messages that told it.
//!
//! A message says what happened; it does not say which order it happened
//! to, beyond the identifiers a venue chose. An order is created under one
//! `ClOrdID`, acknowledged under an `OrderID`, replaced under a new
//! `ClOrdID` that names the old one as `OrigClOrdID`, and every consumer
//! then rebuilds the chain from those references independently - which is
//! how two systems come to disagree about how many orders there were.
//!
//! This module reads a stream once, in order, and stamps every message with
//! three identities the stream implies:
//!
//! | column | holds |
//! | --- | --- |
//! | `instid` | the instrument, the same across venues that spell it alike: the xxh128 digest of its market, classification, ISIN - else symbol - and currency |
//! | `id` | the message: the instant closest to the market impact, then the xxh3 digest of what it said, so ids sort by time and never repeat |
//! | `persistentid` | the order chain: the instant it was created, then the xxh3 digest of its instrument and first identifier, the same on every later message that shares one of its identifiers |
//!
//! # The chain is the identifiers, joined
//!
//! A message joins a chain through any identifier it carries - `OrigClOrdID`
//! first, because a replace names the order it replaces, then `ClOrdID`,
//! `OrderID`, `SecondaryClOrdID` and `SecondaryOrderID` - and every
//! identifier it carries joins the chain in turn, so a replace's new
//! `ClOrdID` reaches the chain the old one opened. A message carrying an
//! identifier no chain holds opens one, dated by its own impact clock.
//!
//! # A chain ends when its state does
//!
//! The [`State`](crate::types::State) a message reports ranks its
//! lifecycle, and a rank past the live ones - filled, done for day,
//! cancelled, rejected, expired - closes the chain: its identifiers are
//! forgotten, so a venue reusing a `ClOrdID` tomorrow opens a new chain
//! rather than joining yesterday's. What is held is therefore the orders
//! still alive, and [`FixLifecycle::alive`] says how many.
//!
//! # The impact clock
//!
//! `TransactTime(60)` is when the venue says it happened; `SendingTime(52)`
//! when the message left; the row's own `timestamp` when the capture saw
//! it. The first stated is the instant an `id` and a `persistentid` carry,
//! in microseconds, so a consumer ordering by id orders by the market's own
//! clock where one was stated.
//!
//! # Nothing here is an entry
//!
//! The three columns are stamped on the row alone. The entries are what
//! arrived, and a message re-emitted after this pass is the received line
//! byte for byte.

use std::collections::HashMap;
use std::sync::Arc;

use smol_str::SmolStr;

use crate::txhash::{TxHash, unix_from_scalar};
use crate::types::State;
use crate::types::ascii::AsciiFamily;
use crate::{DigestAlgorithm, Field, Result, Scalar, TimeUnit};

use super::msg::FixMsg;
use super::registry::FixRegistry;
use super::{ID_TAG, INSTID_TAG, ISINCODE_TAG, MICCODE_TAG, PERSISTENTID_TAG, STATE_TAG};

/// The tags an order is known by, in the order a message is joined on them.
///
/// `OrigClOrdID` first: a replace or a cancel names the order it acts on
/// there, and the new `ClOrdID` it carries is not yet anyone's.
const CHAIN_TAGS: [i32; 5] = [41, 11, 37, 526, 198];

/// The clocks closest to the market impact, most exact first.
const IMPACT_TAGS: [i32; 2] = [60, 52];

/// The separator between the parts an instrument's identity digests.
///
/// A unit separator rather than nothing, so `XNAS` beside `ESVUFR` and
/// `XNASE` beside `SVUFR` are two identities.
const PART_SEPARATOR: u8 = 0x1F;

/// The state a stream of messages has reached, one chain per order alive.
///
/// Built once per stream and fed every message in order through
/// [`fill`](Self::fill); [`FixCodec::lifecycle`](super::FixCodec::lifecycle)
/// does exactly that over an iterator, and the batch reader does it when
/// [`FixOptions::lifecycle`](super::FixOptions::lifecycle) is on.
///
/// ```
/// use std::sync::Arc;
/// use yggdryl::{FixCodec, FixLifecycle, FixRegistry, ID_TAG, PERSISTENTID_TAG};
///
/// # fn main() -> yggdryl::Result<()> {
/// let registry = Arc::new(FixRegistry::new());
/// let reader = FixCodec::new(Arc::clone(&registry));
/// let mut life = FixLifecycle::new(registry);
///
/// // The order, then its acknowledgement under the venue's own identifier.
/// let order = life.fill(reader.transform_line(
///     b"8=FIX.4.4|35=D|11=A1|55=AAPL|54=1|38=100|60=20260102-10:15:30.000|10=0|",
///     false,
/// )?)?;
/// let ack = life.fill(reader.transform_line(
///     b"8=FIX.4.4|35=8|11=A1|37=O1|150=0|39=0|55=AAPL|60=20260102-10:15:30.250|10=0|",
///     false,
/// )?)?;
/// // One chain: the acknowledgement carries the order's persistent id.
/// assert_eq!(order.by_tag(PERSISTENTID_TAG)?, ack.by_tag(PERSISTENTID_TAG)?);
/// // Two messages: two ids, and the later one sorts after.
/// assert!(order.by_tag(ID_TAG)?.as_bytes() < ack.by_tag(ID_TAG)?.as_bytes());
/// assert_eq!(life.alive(), 1);
///
/// // The fill closes the chain, and the venue's identifier is forgotten.
/// life.fill(reader.transform_line(
///     b"8=FIX.4.4|35=8|37=O1|150=F|39=2|14=100|151=0|60=20260102-10:15:31.000|10=0|",
///     false,
/// )?)?;
/// assert_eq!(life.alive(), 0);
/// # Ok(())
/// # }
/// ```
#[derive(Debug)]
pub struct FixLifecycle {
    /// The three columns, resolved once, non-null as a built child is.
    columns: Option<[Field; 3]>,
    /// Every chain ever opened, by index; a closed one is a hole reused.
    chains: Vec<Option<Chain>>,
    /// Which chain each live identifier belongs to.
    keys: HashMap<SmolStr, usize>,
    /// The holes a closed chain left, so the vector stays the size of the
    /// widest moment rather than the whole capture.
    free: Vec<usize>,
}

/// One order still alive.
#[derive(Debug)]
struct Chain {
    /// The identity every message of the chain carries.
    persistent: Scalar,
    /// The identifiers that reach it, forgotten together when it closes.
    keys: Vec<SmolStr>,
}

impl FixLifecycle {
    /// A stream with no order alive yet.
    ///
    /// The three columns are the registry's own crate fields, so a registry
    /// without them - which none built by this crate is - stamps nothing.
    #[must_use]
    pub fn new(registry: Arc<FixRegistry>) -> Self {
        let column = |tag: i32| {
            let mut field = registry.get_field_by_tag(tag)?.clone();
            field.set_nullable(false);
            Some(field)
        };
        let columns = match (column(INSTID_TAG), column(ID_TAG), column(PERSISTENTID_TAG)) {
            (Some(instrument), Some(id), Some(persistent)) => Some([instrument, id, persistent]),
            _ => None,
        };
        Self {
            columns,
            chains: Vec::new(),
            keys: HashMap::new(),
            free: Vec::new(),
        }
    }

    /// How many orders are alive: opened by a message and not yet closed by
    /// a terminal state.
    #[must_use]
    pub fn alive(&self) -> usize {
        self.chains.len() - self.free.len()
    }

    /// Forgets every chain, as a new session or a new day would.
    pub fn clear(&mut self) {
        self.chains.clear();
        self.keys.clear();
        self.free.clear();
    }

    /// Stamps one message with its three identities and moves the chain it
    /// belongs to along.
    ///
    /// A stated value is never overwritten: a message already carrying an
    /// `id` keeps it, and one carrying a `persistentid` joins nothing new,
    /// which is what makes a second pass over a stamped stream a no-op. The
    /// entries are untouched.
    ///
    /// # Errors
    ///
    /// Returns the value contract's refusal when a stamped value does not
    /// fit the column the dictionary declares for it, which the digests
    /// built here cannot provoke.
    pub fn fill(&mut self, message: FixMsg) -> Result<FixMsg> {
        if self.columns.is_none() {
            return Ok(message);
        }
        let stated = |tag: i32| message.get_by_tag(tag).is_some_and(|held| !held.is_null());
        let impact = impact_unix(&message);
        let instrument = instrument_digest(&message);
        let keys = chain_keys(&message);
        // The chain is joined before the columns are borrowed: joining
        // moves the state, stamping only reads it.
        let chain = if stated(PERSISTENTID_TAG) {
            None
        } else {
            self.join(&keys, impact, instrument.as_ref())
        };
        let persistent =
            chain.and_then(|at| self.chains[at].as_ref().map(|held| held.persistent.clone()));
        let mut stamped: Vec<(Field, Scalar)> = Vec::with_capacity(3);
        if let Some([instrument_field, id_field, persistent_field]) = self.columns.as_ref() {
            if !stated(INSTID_TAG) {
                if let Some(held) = &instrument {
                    stamped.push((instrument_field.clone(), Scalar::from(held.to_vec())));
                }
            }
            if !stated(ID_TAG) {
                let digest = DigestAlgorithm::Xxh3.digest(&message.digest().to_be_bytes());
                stamped.push((
                    id_field.clone(),
                    Scalar::from(TxHash::new(impact, digest).into_bytes().to_vec()),
                ));
            }
            if let Some(held) = persistent {
                stamped.push((persistent_field.clone(), held));
            }
        }
        let mut typed: Vec<(Field, Scalar)> = Vec::with_capacity(stamped.len());
        for (field, value) in stamped {
            let held = field.scalar(value)?;
            typed.push((field, held));
        }
        let message = message.appended_many(typed)?;
        if is_terminal(&message) {
            // A stamped stream read again closes the chain its identifiers
            // reach, exactly as the first pass did.
            let closing = chain.or_else(|| keys.iter().find_map(|key| self.keys.get(key).copied()));
            if let Some(at) = closing {
                self.close(at);
            }
        }
        Ok(message)
    }

    /// The chain the identifiers reach, opened where none does.
    ///
    /// Every identifier the message carries then reaches the chain, so an
    /// identifier introduced by a replace joins the order it replaces.
    fn join(
        &mut self,
        keys: &[SmolStr],
        impact: i64,
        instrument: Option<&[u8; 16]>,
    ) -> Option<usize> {
        let first = keys.first()?;
        let at = match keys.iter().find_map(|key| self.keys.get(key).copied()) {
            Some(at) => at,
            None => {
                let persistent = persistent_digest(impact, instrument, first);
                let chain = Chain {
                    persistent,
                    keys: Vec::with_capacity(keys.len()),
                };
                match self.free.pop() {
                    Some(hole) => {
                        self.chains[hole] = Some(chain);
                        hole
                    }
                    None => {
                        self.chains.push(Some(chain));
                        self.chains.len() - 1
                    }
                }
            }
        };
        if let Some(chain) = self.chains[at].as_mut() {
            for key in keys {
                if !self.keys.contains_key(key) {
                    self.keys.insert(key.clone(), at);
                    chain.keys.push(key.clone());
                }
            }
        }
        Some(at)
    }

    /// Forgets one chain and every identifier that reached it.
    fn close(&mut self, at: usize) {
        if let Some(chain) = self.chains[at].take() {
            for key in chain.keys {
                self.keys.remove(&key);
            }
            self.free.push(at);
        }
    }
}

/// The identifiers a message carries, in the order a chain is joined on.
fn chain_keys(message: &FixMsg) -> Vec<SmolStr> {
    let mut keys: Vec<SmolStr> = Vec::with_capacity(CHAIN_TAGS.len());
    for tag in CHAIN_TAGS {
        let Some(text) = message.get_by_tag(tag).and_then(Scalar::as_str) else {
            continue;
        };
        let text = text.trim();
        if text.is_empty() {
            continue;
        }
        // Two tags spelling one identifier - an order acknowledged under the
        // client's own - is one key, not a chain joined to itself.
        if !keys.iter().any(|held| held == text) {
            keys.push(SmolStr::new(text));
        }
    }
    keys
}

/// The instant closest to the market impact, in microseconds since the epoch.
///
/// The venue's transaction time, else the sending time, else the clock the
/// row is dated by - which every built message carries - else the epoch.
fn impact_unix(message: &FixMsg) -> i64 {
    let micros = |held: &Scalar| {
        (!held.is_null())
            .then(|| unix_from_scalar(held, TimeUnit::Microsecond).ok())
            .flatten()
    };
    IMPACT_TAGS
        .into_iter()
        .filter_map(|tag| message.get_by_tag(tag))
        .find_map(micros)
        .or_else(|| micros(&message.market_timestamp()))
        .unwrap_or(0)
}

/// The instrument's identity: the xxh128 digest of its market, its
/// classification, its ISIN - else its symbol - and its currency, or nothing
/// where the message names none of them.
fn instrument_digest(message: &FixMsg) -> Option<[u8; 16]> {
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
    let market = first(&[MICCODE_TAG, 207, 100, 30]);
    let classification = first(&[461]);
    let isin = first(&[ISINCODE_TAG]).or_else(|| {
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
    let mut bytes: Vec<u8> = Vec::with_capacity(32);
    for part in [market, classification, instrument, currency] {
        if let Some(held) = part {
            bytes.extend_from_slice(held.as_bytes());
        }
        bytes.push(PART_SEPARATOR);
    }
    let digest = DigestAlgorithm::Xxh128.digest(&bytes).into_bytes();
    let mut held = [0_u8; 16];
    held.copy_from_slice(&digest[..16]);
    Some(held)
}

/// The chain's identity: the instant it was created, then the xxh3 digest
/// of its instrument and the identifier that opened it.
fn persistent_digest(impact: i64, instrument: Option<&[u8; 16]>, first: &str) -> Scalar {
    let mut bytes: Vec<u8> = Vec::with_capacity(16 + 1 + first.len());
    if let Some(held) = instrument {
        bytes.extend_from_slice(held);
    }
    bytes.push(PART_SEPARATOR);
    bytes.extend_from_slice(first.as_bytes());
    let digest = DigestAlgorithm::Xxh3.digest(&bytes);
    Scalar::from(TxHash::new(impact, digest).into_bytes().to_vec())
}

/// Whether the state the message reports ends the order's life.
///
/// The crate's own `state`, else `OrdStatus`, else `ExecType`, read as the
/// one lifecycle vocabulary; a message stating none is not an ending.
fn is_terminal(message: &FixMsg) -> bool {
    [STATE_TAG, 39, 150]
        .into_iter()
        .filter_map(|tag| message.get_by_tag(tag))
        .find(|held| !held.is_null())
        .and_then(|held| match held {
            Scalar::Ascii(AsciiFamily::State(state)) => Some(state.clone()),
            other => other.as_str().and_then(State::from_spelling),
        })
        .is_some_and(|state| !state.is_live())
}
