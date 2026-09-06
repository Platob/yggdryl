//! The ZIP backend: format records, the index, and the three roles.

use std::sync::Arc;

use smol_str::SmolStr;

use super::{Archive, Entry, Folder, Path, format, name};
use crate::holder::{Buffer, Holder};
use crate::{Codec, Error, IOBase, IOFolder, IOKind, IOPath, Level, MimeType, Url};

/// The archive root of a fresh in-memory archive.
fn root() -> Folder {
    Archive::new(Holder::buffer(Buffer::new())).mount()
}

/// The bytes an archive currently holds, published first.
fn bytes(archive: &Arc<Archive>) -> Vec<u8> {
    archive.image().expect("the archive reads back")
}

/// The archive root over a local path, for the cases that need a location.
fn zip_mount(path: &std::path::Path) -> Holder {
    let root = Holder::zip(Holder::file(path).expect("a local archive"));
    root.child_by_path("x")
        .and_then(|mut child| child.remove(false))
        .ok();
    root
}

/// Mount an archive over an exact byte image.
fn mounted(image: Vec<u8>) -> Folder {
    Archive::new(Holder::buffer(Buffer::from_bytes(image))).mount()
}

/// Assemble an archive image from records this crate would not write itself.
///
/// The refusal cases - an encrypted member, a compression method no build
/// decodes - cannot be produced by the writer, so they are assembled from the
/// format layer directly.
fn image(members: &[(Entry, Vec<u8>)], comment: &[u8]) -> Vec<u8> {
    let mut image = Vec::new();
    let mut placed = Vec::new();
    for (entry, encoded) in members {
        let entry = entry.clone().with_header_offset(image.len() as u64);
        format::write_local(&entry, &mut image);
        image.extend_from_slice(encoded);
        placed.push(entry);
    }
    let directory_offset = image.len() as u64;
    let mut trailer = Vec::new();
    for entry in &placed {
        format::write_central(entry, &mut trailer);
    }
    let directory_size = trailer.len() as u64;
    format::write_end(
        format::End {
            directory_offset,
            directory_size,
            entries: placed.len() as u64,
        },
        comment,
        &mut trailer,
    );
    image.extend_from_slice(&trailer);
    image
}

#[test]
fn a_dos_timestamp_round_trips_to_two_seconds() {
    // 2024-06-03T14:25:36Z, which the two-second DOS clock states exactly.
    let nanos = 1_717_424_736_000_000_000;
    let (date, time) = format::dos_datetime(nanos);
    assert_eq!(format::dos_nanos(date, time), nanos);
}

#[test]
fn a_dos_timestamp_clamps_outside_its_range() {
    // 1970 predates the DOS epoch, and the format cannot state it.
    let (date, time) = format::dos_datetime(0);
    assert_eq!(format::dos_nanos(date, time), 315_532_800_000_000_000);
}

#[test]
fn every_zip_method_maps_to_the_one_codec_dispatcher() {
    for codec in [Codec::Identity, Codec::Deflate, Codec::Zstd] {
        let method = format::method_of(codec).expect("a zip method");
        assert_eq!(format::codec_of(method).expect("a codec"), codec);
    }
    for codec in [Codec::Gzip, Codec::Zlib] {
        assert!(matches!(
            format::method_of(codec),
            Err(Error::Unsupported { .. })
        ));
    }
}

#[test]
fn an_unknown_compression_method_is_named() {
    let error = format::codec_of(14).expect_err("LZMA is not decoded here");
    assert!(
        error.to_string().contains("compression method 14"),
        "{error}"
    );
}

#[test]
fn an_absent_archive_indexes_as_empty() {
    let root = root();
    assert!(root.archive().entries().expect("an empty index").is_empty());
    assert_eq!(root.ls(true, true).count(), 0);
    assert_eq!(root.archive().size(), 0);
    assert!(!root.folder_exists());
}

#[test]
fn a_flush_does_not_create_an_untouched_archive() {
    let root = root();
    root.archive().open().expect("an empty index");
    root.archive().flush().expect("nothing to publish");
    assert_eq!(root.archive().size(), 0);
}

#[test]
fn a_member_round_trips_under_every_supported_coding() {
    let payload = b"symbol,price\nAAPL,187.23\nMSFT,412.10\n".repeat(8);
    for codec in [Codec::Identity, Codec::Deflate, Codec::Zstd] {
        let archive = Arc::new(
            Archive::new(Holder::buffer(Buffer::new()))
                .try_with_codec(codec)
                .expect("a zip coding"),
        );
        archive
            .write_member("trades/eu.csv", &payload)
            .expect("the member writes");
        archive.flush().expect("the directory publishes");

        let entry = archive
            .get_entry("trades/eu.csv")
            .expect("the index")
            .expect("the member");
        assert_eq!(entry.codec().expect("a codec"), codec);
        assert_eq!(entry.size(), payload.len() as u64);
        assert_eq!(
            archive.read_member("trades/eu.csv").expect("the bytes"),
            payload
        );
    }
}

#[test]
fn a_written_archive_is_read_back_after_a_remount() {
    let root = root();
    root.archive()
        .write_member("a/b/c.txt", b"one")
        .expect("the member writes");
    root.archive()
        .write_member("a/d.txt", b"two")
        .expect("the member writes");
    let image = bytes(root.archive());

    let reopened = mounted(image);
    let names: Vec<String> = reopened
        .archive()
        .entries()
        .expect("the index")
        .iter()
        .map(|entry| entry.name().to_owned())
        .collect();
    assert_eq!(names, ["a/b/c.txt", "a/d.txt"]);
    assert_eq!(
        reopened.archive().read_member("a/b/c.txt").expect("bytes"),
        b"one"
    );
}

#[test]
fn a_corrupt_member_fails_its_digest() {
    let root = root();
    root.archive()
        .write_member_with("notes.txt", b"symbol", Codec::Identity)
        .expect("the member writes");
    let mut image = bytes(root.archive());
    // The stored payload sits after the local header, so the first byte that
    // is not header is the member's own.
    let data = format::LOCAL_LEN + "notes.txt".len() + 9;
    image[data] = b'X';

    let error = mounted(image)
        .archive()
        .read_member("notes.txt")
        .expect_err("the digest disagrees");
    assert!(
        matches!(error, Error::Codec { format: "zip", .. }),
        "{error}"
    );
}

#[test]
fn a_replaced_member_reads_as_its_newest_record() {
    let root = root();
    let archive = root.archive();
    archive.write_member("a.txt", b"first").expect("writes");
    archive.write_member("a.txt", b"second").expect("rewrites");
    archive.flush().expect("publishes");

    assert_eq!(archive.entries().expect("the index").len(), 1);
    assert_eq!(archive.read_member("a.txt").expect("bytes"), b"second");
}

#[test]
fn removing_a_member_reclaims_its_bytes() {
    let root = root();
    let archive = root.archive();
    archive
        .write_member_with("big.bin", &vec![7_u8; 4096], Codec::Identity)
        .expect("writes");
    archive
        .write_member_with("small.bin", b"kept", Codec::Identity)
        .expect("writes");
    archive.flush().expect("publishes");
    let before = archive.size();

    assert!(archive.remove_member("big.bin").expect("the removal"));
    archive.flush().expect("publishes");

    assert!(
        archive.size() < before - 4000,
        "expected the removed member's bytes to go, {} -> {}",
        before,
        archive.size()
    );
    assert_eq!(archive.read_member("small.bin").expect("bytes"), b"kept");
    // And the compacted image is still a valid archive from the outside.
    let reopened = mounted(bytes(archive));
    assert_eq!(
        reopened.archive().read_member("small.bin").expect("bytes"),
        b"kept"
    );
}

#[test]
fn compaction_settles_a_streamed_local_header() {
    let payload = b"symbol,price\n";
    let mut crc = flate2::Crc::new();
    crc.update(payload);
    let length = payload.len() as u64;
    let streamed = Entry::new(SmolStr::new("a.txt"), 0, 0)
        .with_flags(format::FLAG_UTF8 | format::FLAG_DATA_DESCRIPTOR)
        .with_content(crc.sum(), length, length);
    let spare = Entry::new(SmolStr::new("b.txt"), 0, 0);
    let mut raw = image(&[(streamed, payload.to_vec()), (spare, Vec::new())], b"");
    // A streaming writer leaves the digest and both sizes out of the local
    // header, promising them after the member's bytes instead.
    raw[14..26].fill(0);

    let root = mounted(raw);
    // Removing anything compacts, which is where a moved record is settled.
    assert!(root.archive().remove_member("b.txt").expect("the removal"));
    root.archive().flush().expect("publishes");

    let settled = bytes(root.archive());
    let word = |at: usize| u32::from_le_bytes(settled[at..at + 4].try_into().expect("four bytes"));
    assert_eq!(
        u16::from_le_bytes([settled[6], settled[7]]) & format::FLAG_DATA_DESCRIPTOR,
        0
    );
    assert_eq!(word(14), crc.sum());
    assert_eq!(word(18), length as u32);
    assert_eq!(word(22), length as u32);
    assert_eq!(root.archive().read_member("a.txt").expect("bytes"), payload);
}

#[test]
fn an_archive_comment_survives_a_rewrite() {
    let root = root();
    let archive = root.archive();
    archive.write_member("a.txt", b"one").expect("writes");
    archive.set_comment(b"day one").expect("the comment");
    let image = bytes(archive);

    let reopened = mounted(image);
    assert_eq!(reopened.archive().comment().expect("a comment"), b"day one");
}

#[test]
fn a_prepended_program_shifts_every_recorded_offset() {
    let root = root();
    root.archive()
        .write_member("a.txt", b"one")
        .expect("writes");
    let mut image = b"#!/bin/sh\nexit 0\n".to_vec();
    image.extend_from_slice(&bytes(root.archive()));

    // The trailer says where the directory really ends, so the whole archive
    // reads at its new position without any recorded offset being right.
    let reopened = mounted(image);
    assert_eq!(
        reopened.archive().read_member("a.txt").expect("bytes"),
        b"one"
    );
}

#[test]
fn a_zip64_trailer_round_trips_its_64_bit_fields() {
    let end = format::End {
        directory_offset: 0x1_0000_0000,
        directory_size: 46,
        entries: 70_000,
    };
    let mut trailer = Vec::new();
    format::write_end(end, b"", &mut trailer);

    // The 32-bit record saturates, and the 64-bit pair before it is the truth.
    let (narrow, _) = format::read_end(&trailer[trailer.len() - format::END_LEN..], 0)
        .expect("the 32-bit record");
    assert_eq!(narrow.entries, u64::from(format::ZIP64_MARK_16));
    assert_eq!(narrow.directory_offset, u64::from(format::ZIP64_MARK_32));

    let wide = format::read_zip64_end(&trailer, 0).expect("the 64-bit record");
    assert_eq!(wide.directory_offset, end.directory_offset);
    assert_eq!(wide.entries, end.entries);
    let locator = format::read_zip64_locator(&trailer[format::ZIP64_END_LEN..], 0)
        .expect("a locator")
        .expect("the record offset");
    assert_eq!(locator, end.directory_offset + end.directory_size);
}

#[test]
fn a_zip64_member_records_its_sizes_in_the_extra_field() {
    // The sizes are what the extra field carries; writing four gigabytes to
    // prove it is not, so the record is assembled and read back directly.
    let entry =
        Entry::new(SmolStr::new("huge.bin"), 0, 0).with_content(1, 0x1_0000_0001, 0x1_0000_0002);
    let mut record = Vec::new();
    format::write_central(&entry, &mut record);
    let mut scan = format::Scan::new(&record, 0);
    let read = format::read_central(&mut scan).expect("the record");
    assert_eq!(read.size(), 0x1_0000_0002);
    assert_eq!(read.compressed_size(), 0x1_0000_0001);
}

#[test]
fn an_encrypted_member_is_refused_by_name() {
    let entry = Entry::new(SmolStr::new("secret.txt"), 0, 0)
        .with_flags(format::FLAG_UTF8 | format::FLAG_ENCRYPTED)
        .with_content(0, 3, 3);
    let root = mounted(image(&[(entry, b"abc".to_vec())], b""));

    let error = root
        .archive()
        .read_member("secret.txt")
        .expect_err("encryption is refused");
    assert!(matches!(error, Error::Unsupported { .. }), "{error}");
}

#[test]
fn member_names_are_canonical_however_they_were_recorded() {
    let entry = Entry::new(SmolStr::new("./a//b.txt"), 0, 0).with_content(0, 3, 3);
    let root = mounted(image(&[(entry, b"abc".to_vec())], b""));
    assert_eq!(
        root.archive()
            .get_entry("a/b.txt")
            .expect("the index")
            .expect("the member")
            .name(),
        "a/b.txt"
    );
}

#[test]
fn a_member_name_cannot_climb_above_the_archive_root() {
    let root = root();
    let error = root
        .child_by_path("../escape.txt")
        .expect_err("the archive root is the top");
    assert!(matches!(error, Error::Parse { .. }), "{error}");
    // But dot segments inside it resolve.
    assert_eq!(
        name::resolve("a/b", "../c/./d.txt").expect("a member path"),
        "a/c/d.txt"
    );
}

#[test]
fn directories_are_implied_by_the_names_of_their_members() {
    let root = root();
    root.child_by_path("2024/06/trades.csv")
        .expect("a member")
        .write_all_bytes(b"symbol")
        .expect("writes");

    let level: Vec<String> = root
        .ls(false, false)
        .map(|entry| entry.expect("an entry").url().expect("a url").to_string())
        .collect();
    assert_eq!(level.len(), 1);
    assert!(level[0].ends_with("#2024"), "{level:?}");

    let tree = root.ls(true, false).count();
    // 2024, 2024/06, and the member itself.
    assert_eq!(tree, 3);
}

#[test]
fn an_explicit_directory_record_lists_as_a_directory() {
    let root = root();
    root.archive().create_directory("empty").expect("a record");

    let entries = root.archive().entries().expect("the index");
    assert_eq!(entries.len(), 1);
    assert!(entries[0].is_directory());
    assert_eq!(entries[0].name(), "empty/");

    let listed: Vec<Holder> = root
        .ls(false, false)
        .map(|entry| entry.expect("an entry"))
        .collect();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].kind(), IOKind::Directory);
}

#[test]
fn private_members_are_hidden_unless_a_listing_asks() {
    let root = root();
    root.child_by_path(".hidden/notes.txt")
        .expect("a member")
        .write_all_bytes(b"x")
        .expect("writes");
    root.child_by_path("open.txt")
        .expect("a member")
        .write_all_bytes(b"y")
        .expect("writes");

    assert_eq!(root.ls(true, false).count(), 1);
    assert_eq!(root.ls(true, true).count(), 3);
}

#[test]
fn a_glob_selects_members_across_levels() {
    let root = root();
    for member in ["2024/06/a.csv", "2024/07/b.csv", "2024/07/c.txt"] {
        root.child_by_path(member)
            .expect("a member")
            .write_all_bytes(b"x")
            .expect("writes");
    }
    assert_eq!(root.glob("**/*.csv", false).expect("a glob").count(), 2);
    assert_eq!(root.glob("2024/07/*", false).expect("a glob").count(), 2);
}

#[test]
fn a_stored_member_reads_positionally_out_of_the_archive() {
    let payload: Vec<u8> = (0..=255_u8).cycle().take(4096).collect();
    let root = root();
    let member = root.as_file("blob.bin").expect("a member");
    root.archive()
        .write_member_with("blob.bin", &payload, Codec::Identity)
        .expect("writes");
    root.archive().flush().expect("publishes");

    assert!(!member.opened(), "a positional read materializes nothing");
    assert_eq!(
        member.read_range_bytes(1000, 16).expect("a range"),
        payload[1000..1016]
    );
    assert_eq!(member.size(), 4096);
    assert!(!member.opened());

    // A read entirely past the end is empty, exactly as any other leaf's is.
    assert!(
        member
            .read_range_bytes(9999, 16)
            .expect("a range")
            .is_empty()
    );
}

#[test]
fn a_compressed_member_reads_positionally_without_being_held() {
    let payload: Vec<u8> = (0..=255_u8).cycle().take(8192).collect();
    let root = root();
    root.archive()
        .write_member_with("blob.bin", &payload, Codec::Deflate)
        .expect("writes");
    root.archive().flush().expect("publishes");

    let member = root.as_file("blob.bin").expect("a member");
    assert_eq!(
        member.read_range_bytes(4000, 32).expect("a range"),
        payload[4000..4032]
    );
    assert!(!member.opened(), "a closed read retains no decoded page");
    assert_eq!(member.read_all_bytes().expect("the member"), payload);
}

#[test]
fn opening_a_member_answers_from_the_value_it_holds() {
    let root = root();
    root.archive()
        .write_member("blob.bin", &vec![3_u8; 1024])
        .expect("writes");
    root.archive().flush().expect("publishes");

    let mut member = root.as_file("blob.bin").expect("a member");
    assert!(!member.opened());
    member.open().expect("the decoded member");
    assert!(member.opened());
    assert_eq!(
        member.read_range_bytes(0, 4).expect("a range"),
        [3, 3, 3, 3]
    );
    member.close().expect("the release");
    assert!(!member.opened());
}

#[test]
fn a_positional_write_grows_the_member_and_zero_fills_the_gap() {
    let root = root();
    let mut member = root.as_file("blob.bin").expect("a member");
    member.pwrite(4, b"tail").expect("a positional write");
    member.flush().expect("publishes");

    assert_eq!(
        root.archive().read_member("blob.bin").expect("bytes"),
        b"\0\0\0\0tail"
    );
}

#[test]
fn a_truncation_shortens_the_member() {
    let root = root();
    let mut member = root.as_file("notes.txt").expect("a member");
    member.write_all_bytes(b"symbol,price").expect("writes");
    member.truncate(6).expect("a truncation");
    member.flush().expect("publishes");

    assert_eq!(
        root.archive().read_member("notes.txt").expect("bytes"),
        b"symbol"
    );
}

#[test]
fn clearing_a_member_empties_it_without_creating_an_absent_one() {
    let root = root();
    let mut absent = root.as_file("absent.txt").expect("a member");
    absent.clear().expect("nothing to empty");
    assert!(root.archive().entries().expect("the index").is_empty());

    let mut member = root.as_file("notes.txt").expect("a member");
    member.write_all_bytes(b"symbol").expect("writes");
    member.clear().expect("the emptying");
    assert_eq!(member.size(), 0);
    assert!(
        root.archive()
            .get_entry("notes.txt")
            .expect("the index")
            .is_some()
    );
}

#[test]
fn removing_a_member_drops_its_pending_write_with_it() {
    let root = root();
    let mut member = root.as_file("notes.txt").expect("a member");
    member.write_all_bytes(b"symbol").expect("writes");
    member.pwrite(0, b"TICKER").expect("a staged write");
    member.remove(false).expect("the removal");
    member.flush().expect("nothing to publish");

    assert!(
        root.archive()
            .get_entry("notes.txt")
            .expect("the index")
            .is_none()
    );
}

#[test]
fn a_directory_refuses_removal_while_it_still_holds_members() {
    let root = root();
    root.child_by_path("lake/a.txt")
        .expect("a member")
        .write_all_bytes(b"x")
        .expect("writes");

    let mut lake = root.as_directory("lake").expect("a directory");
    let error = lake.remove(false).expect_err("a non-empty directory");
    assert!(error.to_string().contains("lake"), "{error}");

    lake.remove(true).expect("the recursive removal");
    assert!(root.archive().entries().expect("the index").is_empty());
}

#[test]
fn clearing_a_directory_keeps_it_and_removes_what_is_under_it() {
    let root = root();
    root.archive().create_directory("lake").expect("a record");
    root.child_by_path("lake/a.txt")
        .expect("a member")
        .write_all_bytes(b"x")
        .expect("writes");
    root.child_by_path("other.txt")
        .expect("a member")
        .write_all_bytes(b"y")
        .expect("writes");

    let mut lake = root.as_directory("lake").expect("a directory");
    lake.clear().expect("the emptying");

    let names: Vec<String> = root
        .archive()
        .entries()
        .expect("the index")
        .iter()
        .map(|entry| entry.name().to_owned())
        .collect();
    assert_eq!(names, ["lake/", "other.txt"]);
}

#[test]
fn the_archive_root_is_a_container_and_its_parent_is_outside() {
    let path = std::env::temp_dir().join("yggdryl-zip-parent.zip");
    let root = Archive::from_path(&path).expect("a local archive").mount();
    assert_eq!(root.kind(), IOKind::Directory);
    assert!(root.is_container());
    assert_eq!(*root.media_type().base(), MimeType::DIRECTORY);

    let parent = root.parent().expect("the folder the archive lives in");
    assert!(parent.is_container());
    assert_ne!(parent.url(), Some(root.url()));
}

#[test]
fn a_member_resolves_to_the_role_it_turns_out_to_be() {
    let root = root();
    let undecided = Path::new(Arc::clone(root.archive()), SmolStr::new("a/b.txt"));
    assert_eq!(undecided.kind(), IOKind::Unknown);
    assert!(!undecided.path_exists());
    assert!(
        undecided
            .read_all_bytes()
            .expect("nothing there")
            .is_empty()
    );

    root.child_by_path("a/b.txt")
        .expect("a member")
        .write_all_bytes(b"x")
        .expect("writes");

    assert_eq!(undecided.kind(), IOKind::File);
    assert!(undecided.is_file());
    let parent = Path::new(Arc::clone(root.archive()), SmolStr::new("a"));
    assert_eq!(parent.kind(), IOKind::Directory);
    assert!(parent.is_folder());
}

#[test]
fn a_member_url_carries_its_path_in_the_fragment() {
    let root = Archive::new(Holder::file("/lake/day.zip").expect("a local archive")).mount();
    let member = root.child_by_path("trades/eu ndx.csv").expect("a member");

    assert_eq!(
        member.url().expect("a url"),
        &Url::from_str("file:///lake/day.zip#trades/eu%20ndx.csv").expect("a url")
    );
    // The archive itself is the same location without a fragment.
    assert_eq!(root.url().to_string(), "file:///lake/day.zip");
    assert!(root.url().fragment(true).expect("no fragment").is_none());
}

#[test]
fn a_member_url_opens_the_member_again() {
    let path = std::env::temp_dir().join("yggdryl-zip-round-trip.zip");
    let root = zip_mount(&path);
    root.child_by_path("trades/eu.csv")
        .expect("a member")
        .write_all_bytes(b"symbol,price")
        .expect("writes");

    let member = root.child_by_path("trades/eu.csv").expect("a member");
    let located = super::from_url(member.url().expect("a member url")).expect("the member reopens");
    assert_eq!(located.read_all_bytes().expect("bytes"), b"symbol,price");
    assert_eq!(located.url(), member.url());

    // And without a fragment the same location is the archive root.
    let archive_url = Url::from_path(&path).expect("a url");
    let reopened = super::from_url(&archive_url).expect("the archive reopens");
    assert!(reopened.is_container());
    assert_eq!(reopened.ls(true, false).count(), 2);

    Holder::file(&path)
        .expect("a handle")
        .remove(false)
        .expect("cleanup");
}

#[test]
fn a_member_url_from_a_scheme_this_backend_cannot_hold_is_refused() {
    let url = Url::from_str("s3://lake/day.zip#trades/eu.csv").expect("a url");
    let error = super::from_url(&url).expect_err("a remote archive");
    assert!(matches!(error, Error::Unsupported { .. }), "{error}");
}

#[test]
fn a_members_representation_comes_from_its_own_name() {
    let root = Archive::new(Holder::file("/lake/day.zip").expect("a local archive")).mount();
    let member = root.child_by_path("logs/app.log.gz").expect("a member");

    // The location's path names the archive, so the member's own name decides.
    assert_eq!(*member.media_type().base(), MimeType::PLAIN_TEXT);
    assert_eq!(member.codec(), Codec::Gzip);
}

#[test]
fn a_member_carries_the_partitions_its_name_and_its_archive_spell() {
    let root =
        Archive::new(Holder::file("/lake/region=eu/day.zip").expect("a local archive")).mount();
    let member = root
        .child_by_path("year=2024/month=01/part-0.parquet")
        .expect("a member");

    assert_eq!(
        member.partitions(),
        vec![
            ("region".to_owned(), "eu".to_owned()),
            ("year".to_owned(), "2024".to_owned()),
            ("month".to_owned(), "01".to_owned()),
        ]
    );
}

#[test]
fn a_member_that_already_carries_a_coding_is_stored_rather_than_recompressed() {
    let root = root();
    let mut member = root.as_file("app.log.gz").expect("a member");
    member
        .write_all_bytes(&crate::coding::gzip::dump(b"symbol").expect("gzip bytes"))
        .expect("writes");

    let entry = root
        .archive()
        .get_entry("app.log.gz")
        .expect("the index")
        .expect("the member");
    assert_eq!(entry.codec().expect("a codec"), Codec::Identity);
}

#[test]
fn an_explicit_member_coding_wins_over_every_default() {
    let root = root();
    let mut member = root
        .as_file("blob.bin")
        .expect("a member")
        .try_with_codec(Codec::Zstd)
        .expect("a zip coding");
    member.write_all_bytes(&vec![1_u8; 512]).expect("writes");

    let entry = root
        .archive()
        .get_entry("blob.bin")
        .expect("the index")
        .expect("the member");
    assert_eq!(entry.codec().expect("a codec"), Codec::Zstd);
}

#[test]
fn a_compression_level_reaches_the_member_writer() {
    let payload = b"symbol,price\n".repeat(256);
    let loose = Arc::new(Archive::new(Holder::buffer(Buffer::new())).with_level(Level::new(1)));
    let tight = Arc::new(Archive::new(Holder::buffer(Buffer::new())).with_level(Level::new(9)));
    for archive in [&loose, &tight] {
        archive.write_member("a.csv", &payload).expect("writes");
        archive.flush().expect("publishes");
    }
    assert!(
        tight.size() <= loose.size(),
        "level 9 should not be larger than level 1"
    );
    assert_eq!(loose.read_member("a.csv").expect("bytes"), payload);
    assert_eq!(tight.read_member("a.csv").expect("bytes"), payload);
}

#[test]
fn a_mounted_archive_walks_as_an_ordinary_container() {
    let root = Holder::zip(Holder::buffer(Buffer::new()));
    root.child_by_path("a/b.txt")
        .expect("a member")
        .write_all_bytes(b"x")
        .expect("writes");

    assert!(root.is_container());
    assert_eq!(root.ls(true, false).count(), 2);
    let child = root.child_by_path("a/b.txt").expect("a member");
    assert!(!child.is_container());
    assert_eq!(child.read_all_bytes().expect("bytes"), b"x");
}

#[test]
fn two_handles_on_one_member_see_the_same_archive() {
    let root = root();
    let mut first = root.as_file("a.txt").expect("a member");
    first.write_all_bytes(b"one").expect("writes");

    let second = root.as_file("a.txt").expect("a member");
    assert_eq!(second.read_all_bytes().expect("bytes"), b"one");

    first.write_all_bytes(b"two").expect("rewrites");
    assert_eq!(second.read_all_bytes().expect("bytes"), b"two");
}

#[test]
fn the_index_is_parsed_once_and_released_by_a_close() {
    let root = root();
    root.archive().write_member("a.txt", b"x").expect("writes");
    root.archive().flush().expect("publishes");

    let mut reopened = mounted(bytes(root.archive()));
    assert!(!reopened.opened());
    reopened.open().expect("the index");
    assert!(reopened.opened());
    reopened.close().expect("the release");
    assert!(!reopened.opened());
}

#[test]
fn mounting_an_archive_reads_its_size_and_its_tail() {
    let root = root();
    root.archive()
        .write_member("a/b.txt", b"one")
        .expect("writes");
    root.archive()
        .write_member("a/c.txt", b"two")
        .expect("writes");
    let image = bytes(root.archive());

    let reopened = mounted(image);
    assert_eq!(
        reopened.archive().handle_reads(),
        0,
        "mounting touches nothing"
    );

    reopened.archive().open().expect("the index");
    // The size, then one tail read the whole directory came out of.
    assert_eq!(reopened.archive().handle_reads(), 2);

    // A listing walks the index, so it asks the handle nothing at all.
    assert_eq!(reopened.ls(true, true).count(), 3);
    assert_eq!(reopened.archive().entries().expect("the index").len(), 2);
    assert_eq!(reopened.archive().handle_reads(), 2);
}

#[test]
fn a_positional_read_of_a_stored_member_is_one_handle_read() {
    let root = root();
    root.archive()
        .write_member_with("blob.bin", &vec![4_u8; 4096], Codec::Identity)
        .expect("writes");
    let reopened = mounted(bytes(root.archive()));
    let member = reopened.as_file("blob.bin").expect("a member");

    // The first read pays for the index and for the member's local header.
    member.read_range_bytes(0, 16).expect("a range");
    let warm = reopened.archive().handle_reads();

    for offset in [64, 1024, 2048] {
        member.read_range_bytes(offset, 16).expect("a range");
    }
    assert_eq!(reopened.archive().handle_reads() - warm, 3);

    // And a second handle on the same member reads no header of its own.
    let second = reopened.as_file("blob.bin").expect("a member");
    let before = reopened.archive().handle_reads();
    second.read_range_bytes(0, 16).expect("a range");
    assert_eq!(reopened.archive().handle_reads() - before, 1);
}

#[test]
fn a_member_this_archive_wrote_needs_no_header_read() {
    let root = root();
    root.archive()
        .write_member_with("blob.bin", &[4_u8; 64], Codec::Identity)
        .expect("writes");
    root.archive().flush().expect("publishes");

    // The write knew where it put the bytes, so reading them back is the read
    // of the bytes and nothing else.
    let before = root.archive().handle_reads();
    root.as_file("blob.bin")
        .expect("a member")
        .read_range_bytes(0, 16)
        .expect("a range");
    assert_eq!(root.archive().handle_reads() - before, 1);
}

#[test]
fn reading_a_stored_member_whole_is_one_handle_read() {
    let root = root();
    root.archive()
        .write_member_with("blob.bin", &vec![4_u8; 4096], Codec::Identity)
        .expect("writes");
    let reopened = mounted(bytes(root.archive()));
    let member = reopened.as_file("blob.bin").expect("a member");

    // Warm the index and the member's data offset.
    member.read_all_bytes().expect("the member");
    let warm = reopened.archive().handle_reads();
    assert_eq!(member.read_all_bytes().expect("the member").len(), 4096);
    assert_eq!(reopened.archive().handle_reads() - warm, 1);
}

#[test]
fn a_parsed_index_answers_the_archive_length() {
    let root = root();
    root.archive()
        .write_member("a.txt", b"one")
        .expect("writes");
    root.archive().flush().expect("publishes");

    let before = root.archive().handle_reads();
    let size = root.archive().size();
    assert!(size > 0);
    assert_eq!(root.archive().size(), size);
    assert_eq!(root.archive().handle_reads(), before);
}

#[test]
fn publishing_writes_one_record_each_and_one_trailer() {
    let root = root();
    for member in 0..5 {
        root.archive()
            .write_member(&format!("part-{member}.csv"), b"symbol")
            .expect("writes");
    }
    // Five records and nothing else: the directory waits for the flush.
    assert_eq!(root.archive().handle_writes(), 5);

    root.archive().flush().expect("publishes");
    // The trailer and the flush behind it; an archive that only grew has
    // nothing past the trailer to discard.
    assert_eq!(root.archive().handle_writes(), 7);

    root.archive().flush().expect("nothing pending");
    assert_eq!(root.archive().handle_writes(), 7);
}

#[cfg(feature = "arrow")]
#[test]
fn a_member_reads_through_the_record_surface() {
    use std::sync::Arc as StdArc;

    use arrow_array::{Int64Array, RecordBatch};

    use crate::{DataType, IOMedia};

    let schema = DataType::from_fields([DataType::Int64.required_field("id")])
        .expect("a struct")
        .required_field("row");
    let arrow_schema = schema.clone().into_arrow_schema().expect("an arrow schema");
    let batch = RecordBatch::try_new(
        StdArc::clone(&arrow_schema),
        vec![StdArc::new(Int64Array::from(vec![7_i64, 9]))],
    )
    .expect("a batch");

    let root = root();
    let mut member = root.child_by_path("trades.arrows").expect("a member");
    let options = member.record_options().expect("an encoding");
    member
        .overwrite_arrow_reader(crate::arrow::batch_reader(arrow_schema, [batch]), &options)
        .expect("the rows write");

    assert_eq!(member.row_size().expect("the row count"), 2);
    // And the folder above it reads as the table beneath it.
    assert!(root.is_tabular());
}
