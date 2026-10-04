//! `rust/src/uri/handle.rs`: an identifier as a handle - the storage it
//! names, resolved on the first operation that needs it and kept for the
//! value's life.

use std::path::PathBuf;

use yggdryl::holder::Holder;
use yggdryl::local::LocalFolder;
use yggdryl::media::RecordOptions;
use yggdryl::{IOBase, IOKind, IOMedia, MimeType, Result, Uri, Url, Urn};

/// A fresh, empty folder of its own under the platform temporary directory.
fn scratch(name: &str) -> Result<PathBuf> {
    let root = LocalFolder::temporary()?
        .path()?
        .join(format!("yggdryl-uri-handle-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root)?;
    Ok(root)
}

#[test]
fn a_location_reads_and_writes_the_storage_it_names() -> Result<()> {
    let root = scratch("location")?;
    let mut uri = Uri::from_path(root.join("trades.csv"))?;

    // Building it touched nothing, and nothing is there yet.
    assert_eq!(IOBase::kind(&uri), IOKind::Unknown);
    assert_eq!(uri.size(), 0);
    assert!(uri.read_all_bytes()?.is_empty());

    uri.write_all_bytes(b"symbol,price\nAAPL,187\nMSFT,410\n")?;
    assert_eq!(IOBase::kind(&uri), IOKind::File);
    assert_eq!(uri.size(), 31);
    assert_eq!(uri.read_range_bytes(0, 6)?, b"symbol");
    assert_eq!(IOBase::media_type(&uri).base(), &MimeType::CSV);

    // The handle is the identifier: its URI is itself, its URL the location.
    assert_eq!(IOBase::uri(&uri), Some(&uri));
    assert_eq!(
        IOBase::url(&uri),
        Some(&Url::from_path(root.join("trades.csv"))?)
    );

    // Records are the resolved handle's, through the media its name declares.
    assert_eq!(IOMedia::row_size(&uri)?, 2);
    let rows = uri
        .read_arrow_reader(&RecordOptions::for_mime_type(&MimeType::CSV)?)?
        .map(|batch| batch.map(|batch| batch.num_rows()))
        .sum::<std::result::Result<usize, _>>()
        .expect("the batches read");
    assert_eq!(rows, 2);

    // A folder is a container, listing what its location holds.
    let folder = Uri::from_path(&root)?;
    assert!(folder.is_container());
    let listed = folder.ls(false, false).collect::<Result<Vec<_>>>()?;
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].url(), IOBase::url(&uri));
    assert_eq!(
        folder.child_by_path("trades.csv")?.read_all_bytes()?.len(),
        31
    );

    std::fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn a_clone_or_a_changed_component_starts_unresolved() -> Result<()> {
    let root = scratch("scope")?;
    let mut uri = Uri::from_path(root.join("ticks.bin"))?;
    uri.write_all_bytes(b"AAPL")?;
    uri.open()?;
    assert!(uri.opened());

    // The resolved handle is no part of the value: equal, but not opened.
    let clone = uri.clone();
    assert_eq!(clone, uri);
    assert!(!clone.opened());

    uri.set_fragment(Some("member"))?;
    assert!(!uri.opened());
    std::fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn a_name_opens_where_it_resolves_and_lends_no_url() -> Result<()> {
    let name = format!(
        "urn:yggdryl:uri-handle:{}:absent.parquet",
        std::process::id()
    );
    let uri = Uri::from_str(&name)?;

    // A name has no URL of its own to lend; its address is itself.
    assert_eq!(IOBase::url(&uri), None);
    assert_eq!(
        IOBase::uri(&uri).map(ToString::to_string),
        Some(name.clone())
    );
    // What it resolves to is the location `Urn::locator` answers, and
    // nothing is there.
    assert_eq!(IOBase::media_type(&uri).base(), &MimeType::PARQUET);
    assert!(uri.read_all_bytes()?.is_empty());
    assert!(!Holder::from(Urn::from_str(&name)?).exists());
    Ok(())
}

#[test]
fn an_identifier_no_backend_holds_is_refused_by_each_operation_that_can_fail() -> Result<()> {
    let mut uri = Uri::from_str("mysql://db.internal/trades")?;
    let refused = uri.read_all_bytes().expect_err("no backend holds mysql");
    assert!(refused.to_string().contains("mysql"), "{refused}");
    assert!(uri.write_all_bytes(b"x").is_err());
    assert!(
        uri.ls(false, false)
            .next()
            .is_some_and(|entry| entry.is_err())
    );

    // An accessor that cannot fail answers the empty answer, and resolving
    // was tried again rather than cached as a failure.
    assert_eq!((uri.size(), IOBase::kind(&uri)), (0, IOKind::Unknown));
    assert_eq!(IOBase::media_type(&uri).base(), &MimeType::OCTET_STREAM);
    // `uri.parent()` is the value's own parent identifier; the handle's is
    // spelled through the trait.
    assert!(IOBase::parent(&uri).is_none() && !uri.opened() && uri.close().is_ok());
    Ok(())
}

#[test]
fn every_identifier_becomes_a_holder_of_what_it_names() -> Result<()> {
    let root = scratch("holder")?;
    let url = Url::from_path(root.join("orders.json"))?;
    let mut held = Holder::from(url.clone());
    assert!(matches!(held, Holder::Uri(_)));
    assert!(!held.exists());

    held.write_all_bytes(br#"{"id":1}"#)?;
    assert!(held.exists());
    assert_eq!(held.url(), Some(&url));
    assert_eq!(
        Holder::from(url.into_uri()).read_all_bytes()?,
        br#"{"id":1}"#
    );
    // An ARN is held as lazily: which backend it needs is its first question.
    let arn = yggdryl::Arn::from_str("arn:aws:s3:::lake/orders.json")?;
    assert!(matches!(Holder::from(arn), Holder::Uri(_)));

    std::fs::remove_dir_all(root)?;
    Ok(())
}

/// A table bucket's ARN used as a handle is the bucket's catalog: resolving
/// it builds a description, so nothing is sent and nothing is read.
#[cfg(feature = "s3tables")]
#[test]
fn a_table_bucket_arn_is_the_catalog_it_names() -> Result<()> {
    let arn = yggdryl::Arn::from_str("arn:aws:s3tables:eu-west-3:123456789012:bucket/lake")?;
    let uri = arn.into_uri();
    assert_eq!(IOBase::kind(&uri), IOKind::Catalog);
    assert!(uri.is_container());

    // Held as a `Holder` it is the same identifier, resolved the same way.
    let Holder::Uri(held) = Holder::from(uri.clone()) else {
        panic!("expected an identifier held as it is");
    };
    assert_eq!(IOBase::kind(&held), IOKind::Catalog);

    // A name lends no URL of its own: the ARN is the address.
    assert_eq!(IOBase::url(&uri), None);
    assert_eq!(IOBase::uri(&uri), Some(&uri));
    Ok(())
}
