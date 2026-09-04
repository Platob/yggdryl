//! Filename extensions and media suffix handling.

use super::*;

pub(super) fn normalize_extension(value: &str) -> Result<SmolStr> {
    if value.is_empty() {
        return Err(parse_error(
            "uri extension",
            0,
            "filename extension must not be empty",
        ));
    }
    if let Some(position) = value.find('.') {
        return Err(parse_error(
            "uri extension",
            position,
            "filename extension must not contain a dot",
        ));
    }
    normalize_resource_segment(
        value,
        "uri extension",
        "filename extension must not be empty",
    )
}

pub(super) fn preferred_mime_extension(value: &MimeType) -> Result<&'static str> {
    value.extension().ok_or_else(|| {
        parse_error(
            "MIME type",
            0,
            "MIME type has no preferred filename extension",
        )
    })
}

/// A cloning, allocation-free iterator over compound filename extensions.
#[derive(Clone, Debug)]
pub struct Extensions<'a> {
    inner: Option<std::str::Split<'a, char>>,
    prefixes_to_skip: u8,
}

impl<'a> Iterator for Extensions<'a> {
    type Item = &'a str;

    fn next(&mut self) -> Option<Self::Item> {
        let inner = self.inner.as_mut()?;
        while self.prefixes_to_skip > 0 {
            inner.next();
            self.prefixes_to_skip -= 1;
        }
        inner.find(|extension| !extension.is_empty())
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (0, self.inner.as_ref().and_then(|inner| inner.size_hint().1))
    }
}

impl FusedIterator for Extensions<'_> {}

pub(super) fn extension_from_file_name(file_name: &str) -> Option<&str> {
    let (stem, extension) = file_name.rsplit_once('.')?;
    (!stem.is_empty() && !extension.is_empty()).then_some(extension)
}

pub(super) fn stem_from_file_name(file_name: &str) -> &str {
    extension_from_file_name(file_name).map_or(file_name, |extension| {
        &file_name[..file_name.len() - extension.len() - 1]
    })
}

pub(super) fn extensions_from_file_name(file_name: Option<&str>) -> Extensions<'_> {
    let (inner, prefixes_to_skip) = file_name.map_or((None, 0), |name| {
        let hidden = name.starts_with('.');
        let search = if hidden { &name[1..] } else { name };
        let has_extension = search
            .find('.')
            .is_some_and(|position| position + 1 < search.len());
        if has_extension {
            (Some(name.split('.')), if hidden { 2 } else { 1 })
        } else {
            (None, 0)
        }
    });
    Extensions {
        inner,
        prefixes_to_skip,
    }
}

pub(super) fn compound_extension_start(file_name: &str) -> Option<usize> {
    let search_start = usize::from(file_name.starts_with('.'));
    let search = &file_name[search_start..];
    search
        .find('.')
        .filter(|position| position + 1 < search.len())
        .map(|position| search_start + position)
}
