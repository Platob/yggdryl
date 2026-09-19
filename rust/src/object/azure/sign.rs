//! Azure's Shared Key signature, and the date every request states.
//!
//! Azure signs a fixed list of headers in a fixed order, then every `x-ms-`
//! header sorted, then the resource the request addresses. The order is not
//! alphabetical and is load-bearing - `If-Modified-Since` comes before
//! `If-Match` - so it is written out here exactly as the service verifies it.
//!
//! Pure: `std`, `hmac` and `sha2` only, the same two primitives Signature
//! Version 4 uses.

use base64::Engine as _;
use hmac::{Hmac, KeyInit, Mac};
use sha2::Sha256;

use crate::{Error, Result};

/// The header lines the signature covers, in the order it covers them.
///
/// `content-length` is the one with a rule of its own: a zero-length body signs
/// as the empty string rather than as `0`.
const SIGNED: [&str; 11] = [
    "content-encoding",
    "content-language",
    "content-length",
    "content-md5",
    "content-type",
    "date",
    "if-modified-since",
    "if-match",
    "if-none-match",
    "if-unmodified-since",
    "range",
];

/// One account's shared key, bound to the account it signs for.
pub(crate) struct SharedKey {
    account: String,
    /// The raw key, decoded from the base64 Azure hands out. Never rendered.
    key: Vec<u8>,
}

impl SharedKey {
    /// Hold `key`, which is base64 exactly as the portal shows it.
    ///
    /// # Errors
    ///
    /// Returns a refusal when the key is not base64.
    pub(crate) fn new(account: &str, key: &str) -> Result<Self> {
        let key = base64::engine::general_purpose::STANDARD
            .decode(key.trim())
            .map_err(|error| {
                Error::Io(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    format!("expected a base64 Azure account key: {error}"),
                ))
            })?;
        Ok(Self {
            account: account.to_owned(),
            key,
        })
    }

    /// The headers to add to the request: the date, then the authorization.
    ///
    /// * `method`: the HTTP verb as sent.
    /// * `path`: the request path exactly as sent, escapes kept.
    /// * `query`: raw, undecoded name/value pairs.
    /// * `headers`: every header the request already carries.
    /// * `now`: the signing time, which is also what `x-ms-date` states.
    pub(crate) fn sign(
        &self,
        method: &str,
        path: &str,
        query: &[(String, String)],
        headers: &[(String, String)],
        now: std::time::SystemTime,
    ) -> Vec<(String, String)> {
        let date = http_date(now);
        let mut signed: Vec<(String, String)> = headers
            .iter()
            .map(|(name, value)| (name.to_ascii_lowercase(), value.clone()))
            .collect();
        signed.push(("x-ms-date".to_owned(), date.clone()));
        let string_to_sign = self.string_to_sign(method, path, query, &signed);
        let mut mac = <Hmac<Sha256> as KeyInit>::new_from_slice(&self.key)
            .expect("HMAC accepts a key of any length");
        mac.update(string_to_sign.as_bytes());
        let signature =
            base64::engine::general_purpose::STANDARD.encode(mac.finalize().into_bytes());
        vec![
            ("x-ms-date".to_owned(), date),
            (
                "authorization".to_owned(),
                format!("SharedKey {}:{signature}", self.account),
            ),
        ]
    }

    /// The document the signature is taken over.
    fn string_to_sign(
        &self,
        method: &str,
        path: &str,
        query: &[(String, String)],
        headers: &[(String, String)],
    ) -> String {
        let value = |wanted: &str| {
            headers
                .iter()
                .find(|(name, _)| name == wanted)
                .map(|(_, value)| collapse(value))
                .unwrap_or_default()
        };
        let mut document = String::with_capacity(256);
        document.push_str(&method.to_ascii_uppercase());
        document.push('\n');
        for name in SIGNED {
            let held = match name {
                // A zero-length body signs as nothing at all.
                "content-length" => {
                    let held = value(name);
                    if held == "0" { String::new() } else { held }
                }
                // The date is in `x-ms-date`, which the canonical headers
                // already carry; signing it twice is what the service refuses.
                "date" => String::new(),
                _ => value(name),
            };
            document.push_str(&held);
            document.push('\n');
        }
        document.push_str(&canonical_headers(headers));
        document.push_str(&self.canonical_resource(path, query));
        document
    }

    /// The resource line: the account, the path as sent, then the query.
    fn canonical_resource(&self, path: &str, query: &[(String, String)]) -> String {
        let mut resource = format!(
            "/{}{}",
            self.account,
            if path.is_empty() { "/" } else { path }
        );
        // Names are lowercased and sorted; a name given twice contributes its
        // values sorted and joined, which is what a repeated parameter means.
        let mut names: Vec<String> = query
            .iter()
            .map(|(name, _)| name.to_ascii_lowercase())
            .collect();
        names.sort();
        names.dedup();
        for name in names {
            let mut values: Vec<String> = query
                .iter()
                .filter(|(held, _)| held.to_ascii_lowercase() == name)
                .map(|(_, value)| value.clone())
                .collect();
            values.sort();
            resource.push('\n');
            resource.push_str(&name);
            resource.push(':');
            resource.push_str(&values.join(","));
        }
        resource
    }
}

/// The `x-ms-` headers, lowercased, sorted, one line each.
fn canonical_headers(headers: &[(String, String)]) -> String {
    let mut owned: Vec<(String, String)> = headers
        .iter()
        .filter(|(name, _)| name.starts_with("x-ms-"))
        .map(|(name, value)| (name.clone(), collapse(value)))
        .collect();
    owned.sort_by(|left, right| left.0.cmp(&right.0));
    owned.dedup_by(|later, first| {
        if later.0 == first.0 {
            first.1 = format!("{},{}", first.1, later.1);
            true
        } else {
            false
        }
    });
    owned
        .into_iter()
        .map(|(name, value)| format!("{name}:{value}\n"))
        .collect()
}

/// One header value with its internal whitespace runs collapsed to a space.
fn collapse(value: &str) -> String {
    let mut collapsed = String::with_capacity(value.len());
    let mut spaced = false;
    for character in value.trim().chars() {
        if character.is_whitespace() {
            if !spaced {
                collapsed.push(' ');
                spaced = true;
            }
        } else {
            collapsed.push(character);
            spaced = false;
        }
    }
    collapsed
}

/// The instant `now`, as RFC 7231 spells a date in a header.
///
/// Azure states its own clock in `x-ms-date` and refuses a request more than
/// fifteen minutes from it, so this is the one rendering that matters.
pub(crate) fn http_date(now: std::time::SystemTime) -> String {
    const DAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let seconds = now
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let days = seconds / 86_400;
    let (year, month, day) = super::super::sigv4::civil_from_days(days);
    // 1970-01-01 was a Thursday, which is index 4.
    let weekday = DAYS[usize::try_from((days + 4) % 7).unwrap_or(0)];
    let month_name = MONTHS[usize::try_from(month.saturating_sub(1))
        .unwrap_or(0)
        .min(11)];
    let (hour, minute, second) = (seconds / 3600 % 24, seconds / 60 % 60, seconds % 60);
    format!("{weekday}, {day:02} {month_name} {year:04} {hour:02}:{minute:02}:{second:02} GMT")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, UNIX_EPOCH};

    fn key() -> SharedKey {
        // Azurite's published development key, which is not a secret.
        SharedKey::new(
            "devstoreaccount1",
            "Eby8vdM02xNOcqFlqUwJPLlmEtlCDXJ1OUzFT50uSRZ6IFsuFq2UVErCz4I6tq/K1SZFPTOtr/KBHBeksoGMGw==",
        )
        .expect("a base64 key")
    }

    fn pairs(list: &[(&str, &str)]) -> Vec<(String, String)> {
        list.iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect()
    }

    #[test]
    fn a_date_renders_the_way_a_header_spells_one() {
        assert_eq!(http_date(UNIX_EPOCH), "Thu, 01 Jan 1970 00:00:00 GMT");
        assert_eq!(
            http_date(UNIX_EPOCH + Duration::from_secs(1_700_000_000)),
            "Tue, 14 Nov 2023 22:13:20 GMT"
        );
    }

    #[test]
    fn the_signed_document_keeps_the_order_the_service_verifies() {
        let document = key().string_to_sign(
            "GET",
            "/lake/part.parquet",
            &pairs(&[("comp", "list"), ("restype", "container")]),
            &pairs(&[
                ("x-ms-version", "2025-05-05"),
                ("x-ms-date", "Thu, 01 Jan 1970 00:00:00 GMT"),
                ("content-length", "0"),
            ]),
        );
        assert_eq!(
            document,
            "GET\n\n\n\n\n\n\n\n\n\n\n\n\
             x-ms-date:Thu, 01 Jan 1970 00:00:00 GMT\nx-ms-version:2025-05-05\n\
             /devstoreaccount1/lake/part.parquet\ncomp:list\nrestype:container"
        );
    }

    #[test]
    fn a_zero_length_body_signs_as_nothing_and_a_real_one_signs_as_its_length() {
        let signed = |length: &str| {
            key().string_to_sign(
                "PUT",
                "/lake/part.bin",
                &[],
                &pairs(&[("content-length", length), ("x-ms-date", "d")]),
            )
        };
        assert!(signed("0").starts_with("PUT\n\n\n\n"), "{}", signed("0"));
        assert!(
            signed("512").starts_with("PUT\n\n\n512\n"),
            "{}",
            signed("512")
        );
    }

    #[test]
    fn every_x_ms_header_is_signed_sorted_and_whitespace_collapsed() {
        let document = key().string_to_sign(
            "PUT",
            "/lake/part.bin",
            &[],
            &pairs(&[
                ("x-ms-meta-desk", "  power   trading "),
                ("x-ms-blob-type", "BlockBlob"),
                ("x-ms-date", "d"),
                ("authorization", "should not be signed"),
            ]),
        );
        let block = document
            .split_once("x-ms-blob-type")
            .expect("the first x-ms header")
            .1;
        assert!(
            block.starts_with(":BlockBlob\nx-ms-date:d\nx-ms-meta-desk:power trading\n"),
            "{document}"
        );
        assert!(!document.contains("should not be signed"));
    }

    #[test]
    fn the_signature_is_a_header_and_the_key_is_never_in_it() {
        let headers = key().sign("GET", "/lake", &[], &[], UNIX_EPOCH);
        let authorization = headers
            .iter()
            .find(|(name, _)| name == "authorization")
            .expect("an authorization header");
        assert!(authorization.1.starts_with("SharedKey devstoreaccount1:"));
        assert!(!authorization.1.contains("Eby8vdM02xNOcqFlqUwJPLlmEtlCDXJ1"));
    }

    #[test]
    fn a_repeated_query_name_contributes_its_values_sorted() {
        let resource = key().canonical_resource(
            "/lake",
            &pairs(&[("include", "snapshots"), ("include", "metadata")]),
        );
        assert_eq!(
            resource,
            "/devstoreaccount1/lake\ninclude:metadata,snapshots"
        );
    }
}
