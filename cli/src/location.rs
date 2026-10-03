//! Where a command's data lives, as a terminal spells it.
//!
//! Every serving command names what it serves the same way: `name=location`,
//! or a location alone named after its last segment, the location a folder
//! path or a URL a [`Holder`] resolves. One reading here, so `yggdryl xmla
//! serve market=/data/market` and `yggdryl market serve books=/data/books` read
//! their arguments by the same rule and refuse the same spellings: the one
//! [`Url::from_location`] reads for a binding's `location`, so text carrying a
//! scheme is a URL - `file:/data/x` as much as `file:///data/x` - and a
//! malformed one is refused as a URL rather than read as a folder named after
//! it, and text carrying none is a path rooted at the working directory.

use std::path::PathBuf;

use yggdryl::holder::Holder;
use yggdryl::{Result, Url};

/// `name=location`, or a location alone named after its last segment: the
/// name and the location text, the holder not yet resolved.
///
/// A name holds no `/`, `\` or `:`, so `C:\data` and `s3://bucket/market`
/// are locations named after their last segment rather than an empty name
/// beside a drive letter or a scheme.
#[must_use]
pub fn split(spelled: &str) -> (String, &str) {
    match spelled.split_once('=') {
        Some((name, location)) if !name.is_empty() && !name.contains(['/', '\\', ':']) => {
            (name.to_owned(), location)
        }
        _ => (last_segment(spelled), spelled),
    }
}

/// [`split`], with the location resolved to the folder it names.
///
/// # Errors
///
/// Returns what [`folder`] refuses.
pub fn named_folder(spelled: &str) -> Result<(String, Holder)> {
    let (name, location) = split(spelled);
    Ok((name, folder(location)?))
}

/// The last segment of a path or a URL, which names a bare location.
#[must_use]
pub fn last_segment(location: &str) -> String {
    location
        .trim_end_matches(['/', '\\'])
        .rsplit(['/', '\\'])
        .next()
        .filter(|segment| !segment.is_empty())
        .unwrap_or(location)
        .to_owned()
}

/// A folder path, or a URL a holder resolves.
///
/// # Errors
///
/// Returns text the location door refuses, or a path that cannot be spelled
/// as a canonical `file:` URL.
pub fn folder(location: &str) -> Result<Holder> {
    folder_of(&Url::from_location(location)?)
}

/// A file path, or a URL a holder resolves.
///
/// # Errors
///
/// Returns text the location door refuses, or a path that cannot be spelled
/// as a canonical `file:` URL.
pub fn file(location: &str) -> Result<Holder> {
    file_of(&Url::from_location(location)?)
}

/// The folder a location already read names.
///
/// # Errors
///
/// Returns what [`from_url`] refuses, or a `file:` URL no platform path
/// spells.
pub fn folder_of(url: &Url) -> Result<Holder> {
    pinned(url, Holder::folder)
}

/// The file a location already read names.
///
/// # Errors
///
/// Returns what [`from_url`] refuses, or a `file:` URL no platform path
/// spells.
pub fn file_of(url: &Url) -> Result<Holder> {
    pinned(url, Holder::file)
}

/// Whether `url` names a place on this machine, whose role the caller states
/// rather than the holder learns: a `file:` URL that names no archive member.
///
/// # Errors
///
/// Returns the URL's refusal of its own fragment.
pub fn is_place(url: &Url) -> Result<bool> {
    Ok(url.is_local()
        && url
            .fragment(false)?
            .is_none_or(|fragment| fragment.is_empty()))
}

/// The holder a URL names, with no role stated and no properties beside the
/// URL's own.
///
/// # Errors
///
/// Returns a scheme no backend of this build holds.
pub fn from_url(url: &Url) -> Result<Holder> {
    Holder::from_url(url, std::iter::empty::<(String, String)>())
}

/// A place on this machine takes the role the caller states, its path rooted
/// where the URL says; anything else is the holder its URL names.
fn pinned(url: &Url, local: fn(PathBuf) -> Result<Holder>) -> Result<Holder> {
    if is_place(url)? {
        return local(url.clone().into_path()?);
    }
    from_url(url)
}
