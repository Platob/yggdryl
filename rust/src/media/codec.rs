//! What every record medium states about itself once, and the register
//! every intake door reads it from.
//!
//! A medium is a [`MediaCodec`]: one `static` in the medium's own file naming
//! its MIME types, building its default options, decoding and encoding one
//! leaf, and opening the stateful wrapper, claimed on [`Register`] under each
//! MIME type it names. Intake - a handle's media type, a `for_mime_type` call,
//! `Media::open` - reads the register once through [`codec_for`]; past it a
//! [`RecordOptions`] in hand answers its medium through
//! [`RecordOptions::codec`], so no leaf, batch or row read looks a medium up.
//! The core claims its own media before the register answers anything, until
//! each leaving crate's `install()` claims its own; a type no claim answers is
//! refused naming the crate to install.

use std::fmt;
use std::sync::{Mutex, OnceLock};

use smol_str::{SmolStr, format_smolstr};

use crate::arrow::BatchReader;
use crate::holder::Holder;
use crate::media::{IORecordOptions, Media, RecordOptions};
use crate::plugin::{CORE, Register};
use crate::text::expected_got;
use crate::{Error, Field, IOBase, MimeType, Result, StreamSerie};

/// The rank a medium outside the core takes at least: the core's seven hold
/// the positions below it, in the order their options sort.
pub const EXTERNAL_RANK: u8 = 32;

/// One record medium, stated once as a `static` in its own file.
///
/// The leaf doors take the handle as the one storage trait and the options
/// whole, reading their own settings back through
/// [`RecordOptions::settings`]; `handle` is never a container.
pub trait MediaCodec: fmt::Debug + Send + Sync + 'static {
    /// The medium's name and the tag its options hash under: `parquet`.
    fn name(&self) -> &'static str;

    /// How a refusal names the medium: `Parquet`.
    fn title(&self) -> &'static str;

    /// Where the medium's options sort among every medium's: the core's are
    /// `ipc` 0, `parquet` 1, `avro` 2, `text` 3, `xmla` 4, `csv` 5 and
    /// `excel` 6; a medium outside the core takes [`EXTERNAL_RANK`] or more.
    fn rank(&self) -> u8;

    /// The MIME types the medium answers, the first canonical.
    fn mime_types(&self) -> &'static [MimeType];

    /// The default options of the medium under `base`, one of
    /// [`Self::mime_types`].
    fn default_options(&self, base: &MimeType) -> RecordOptions;

    /// Whether the medium compresses inside its own container, so an outer
    /// content coding names a file no reader of the medium opens.
    fn compresses_internally(&self) -> bool {
        false
    }

    /// Whether a stored row has an identity a merge can match on; plain
    /// text lines have none.
    fn has_row_identity(&self) -> bool {
        true
    }

    /// Decode one leaf into batches, `declared` pushed down where the medium
    /// projects.
    ///
    /// # Errors
    ///
    /// Returns a read or decoding failure.
    fn read_batch_reader(
        &self,
        handle: &dyn IOBase,
        declared: Option<&Field>,
        options: &RecordOptions,
    ) -> Result<BatchReader>;

    /// Count one leaf's rows from its metadata, decoding no row.
    ///
    /// # Errors
    ///
    /// Returns a read or metadata failure.
    fn row_size(&self, handle: &dyn IOBase, options: &RecordOptions) -> Result<u64>;

    /// Read one leaf's canonical Struct root from its metadata.
    ///
    /// # Errors
    ///
    /// Returns a read or metadata failure.
    fn read_field(&self, handle: &dyn IOBase, options: &RecordOptions) -> Result<Field>;

    /// The root a non-empty leaf's own bytes declare, `None` where it holds
    /// no shape yet: by default the probe under the medium's default options
    /// and the options' root name.
    ///
    /// # Errors
    ///
    /// Returns a read or metadata failure.
    fn stated_field(&self, handle: &dyn IOBase, options: &RecordOptions) -> Result<Option<Field>> {
        let mut probe = self.default_options(&options.mime_type());
        probe.set_name(SmolStr::new(options.name()));
        Ok(Some(self.read_field(handle, &probe)?))
    }

    /// The medium's native row stream over one leaf, `None` where it decodes
    /// batches alone.
    ///
    /// # Errors
    ///
    /// Returns a read or decoding failure.
    fn read_stream(
        &self,
        handle: &dyn IOBase,
        declared: Option<&Field>,
        options: &RecordOptions,
    ) -> Result<Option<StreamSerie>> {
        let _ = (handle, declared, options);
        Ok(None)
    }

    /// Encode `batches` as one leaf's whole contents.
    ///
    /// # Errors
    ///
    /// Returns an encoding or write failure.
    fn overwrite_arrow_reader(
        &self,
        handle: &mut dyn IOBase,
        batches: BatchReader,
        options: &RecordOptions,
    ) -> Result<()>;

    /// The medium's stateful wrapper over `handle`.
    fn open(&self, handle: Holder) -> Media;
}

/// The stateful wrapper a registered medium holds a [`Holder`] in, as
/// [`Media::Registered`] carries it: a byte handle answering the record
/// surface, naming its codec and its handle.
pub trait MediaWrapper: IOBase + Sync + fmt::Debug {
    /// The medium this wrapper encodes - `medium`, because `codec` is the
    /// content coding every [`IOBase`] answers.
    fn medium(&self) -> &'static dyn MediaCodec;

    /// Borrow the byte handle the wrapper reads and writes through.
    fn handle(&self) -> &Holder;

    /// Consume the wrapper and return its byte handle.
    fn into_handle(self: Box<Self>) -> Holder;

    /// The wrapper with an explicit canonical schema.
    fn with_field(self: Box<Self>, field: Field) -> Box<dyn MediaWrapper>;
}

static CODECS: Register<MimeType, &'static dyn MediaCodec> = Register::new("record medium");
static SEEDED: OnceLock<()> = OnceLock::new();
/// One claim at a time, so a codec's MIME types are claimed all or none.
static CLAIMING: Mutex<()> = Mutex::new(());

/// Claim the core's own media once, before the register answers anything.
fn seed() {
    SEEDED.get_or_init(|| {
        let core: &[&'static dyn MediaCodec] = &[
            &crate::ipc::IPC_CODEC,
            #[cfg(feature = "parquet")]
            &crate::parquet::PARQUET_CODEC,
            &crate::avro::AVRO_CODEC,
            &crate::text::TEXT_CODEC,
            &crate::xmla::XMLA_CODEC,
            &crate::csv::CSV_CODEC,
            &crate::excel::EXCEL_CODEC,
        ];
        for &codec in core {
            // The core's claims cannot conflict: each MIME type is stated
            // once in the crate.
            claim_unseeded(codec, CORE).expect("the core's own media claim cleanly");
        }
    });
}

/// Claim `codec` for the crate `by`: every MIME type it names, once for the
/// life of the process.
///
/// # Errors
///
/// Returns [`Error::Conflict`] naming the first claimant where a MIME type
/// is claimed already, and [`Error::InvalidRecord`] at `$.encoding` for a
/// claim in the core's own name, a codec naming no MIME type, or a rank
/// below [`EXTERNAL_RANK`].
pub fn claim(codec: &'static dyn MediaCodec, by: &'static str) -> Result<()> {
    seed();
    if by == CORE {
        return Err(invalid(format_smolstr!(
            "a medium is claimed by the crate that holds it, never as `{CORE}`"
        )));
    }
    if codec.rank() < EXTERNAL_RANK {
        return Err(invalid(format_smolstr!(
            "a medium outside the core ranks at or above {EXTERNAL_RANK}, got {} for `{}`",
            codec.rank(),
            codec.name()
        )));
    }
    claim_unseeded(codec, by)
}

fn claim_unseeded(codec: &'static dyn MediaCodec, by: &'static str) -> Result<()> {
    let _claiming = CLAIMING
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if codec.mime_types().is_empty() {
        return Err(invalid(format_smolstr!(
            "a medium names at least one MIME type, `{}` names none",
            codec.name()
        )));
    }
    // Every type checked before any is claimed, so a refused claim leaves
    // the register as it was.
    for base in codec.mime_types() {
        if let Some(first) = CODECS.claimant(base) {
            return Err(Error::Conflict {
                expected: "record medium",
                actual: first,
                path: SmolStr::new(base.as_str()),
            });
        }
    }
    // One name per medium, whatever types it claims: the name is the tag its
    // options hash under and what orders two media of one rank, so a second
    // claim of it is refused naming the first.
    if let Some(held) = CODECS
        .values()
        .into_iter()
        .find(|held| held.name() == codec.name())
    {
        return Err(Error::Conflict {
            expected: "record medium",
            actual: held
                .mime_types()
                .first()
                .and_then(|base| CODECS.claimant(base))
                .unwrap_or(CORE),
            path: SmolStr::new(codec.name()),
        });
    }
    for base in codec.mime_types() {
        CODECS.claim(base.clone(), codec, by)?;
    }
    Ok(())
}

fn invalid(reason: SmolStr) -> Error {
    Error::InvalidRecord {
        path: SmolStr::new_static("$.encoding"),
        reason,
    }
}

/// The medium claimed under `base`, if any: the lookup every intake door
/// asks, which allocates nothing on a miss.
#[must_use]
pub fn codec_of(base: &MimeType) -> Option<&'static dyn MediaCodec> {
    seed();
    CODECS.get(base)
}

/// The medium claimed under `base`.
///
/// # Errors
///
/// Returns [`Error::InvalidRecord`] at `$` naming the media this build
/// implements and the crate to install where no claim answers `base`.
pub fn codec_for(base: &MimeType) -> Result<&'static dyn MediaCodec> {
    codec_of(base).ok_or_else(|| unregistered(base))
}

/// Every claimed medium, in rank order.
#[must_use]
pub fn codecs() -> Vec<&'static dyn MediaCodec> {
    seed();
    let mut codecs = CODECS.values();
    codecs.sort_by_key(|codec| (codec.rank(), codec.name()));
    codecs.dedup_by_key(|codec| codec.name());
    codecs
}

/// What the refusal of an unclaimed type names: every claimed MIME type in
/// rank order, and the crate to install.
pub(crate) fn implemented() -> String {
    let claimed: Vec<String> = codecs()
        .iter()
        .flat_map(|codec| codec.mime_types())
        .map(ToString::to_string)
        .collect();
    format!(
        "a record encoding this build implements ({}; install the crate that claims it and call \
         its `install()`)",
        claimed.join(", ")
    )
}

/// The refusal of `base`, a MIME type no claim answers.
pub(crate) fn unregistered(base: &MimeType) -> Error {
    Error::InvalidRecord {
        path: SmolStr::new_static("$"),
        reason: expected_got(implemented(), base),
    }
}
