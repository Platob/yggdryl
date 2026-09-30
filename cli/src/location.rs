//! Where a command's data lives, as a terminal spells it.
//!
//! Every serving command names what it serves the same way: `name=location`,
//! or a location alone named after its last segment, the location a folder
//! path or a URL a [`Holder`] resolves. One reading here, so `yggdryl xmla
//! serve market=/data/market` and `yggdryl market serve books=/data/books` read
//! their arguments by the same rule and refuse the same spellings.

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
/// Returns a URL the grammar refuses, or a path that cannot be spelled as a
/// canonical `file:` URL.
pub fn folder(location: &str) -> Result<Holder> {
    // `C:\data` spells a drive, not a scheme, so a URL is one with a `//`.
    if location.contains("://") {
        return from_url(location);
    }
    Holder::folder(location)
}

/// A file path, or a URL a holder resolves.
///
/// # Errors
///
/// Returns a URL the grammar refuses, or a path that cannot be spelled as a
/// canonical `file:` URL.
pub fn file(location: &str) -> Result<Holder> {
    if location.contains("://") {
        return from_url(location);
    }
    Holder::file(location)
}

/// The holder a URL names, with no properties beside the URL's own.
fn from_url(location: &str) -> Result<Holder> {
    let url = Url::from_str(location)?;
    Holder::from_url(&url, std::iter::empty::<(String, String)>())
}
