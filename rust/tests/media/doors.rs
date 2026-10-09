//! `rust/src/media/doors.rs`: the doors a medium implemented outside the
//! crate answers `IOMedia` through - the core's own, reached by name.

use yggdryl::holder::Buffer;
use yggdryl::media::doors::{dimension_options, own_options};
use yggdryl::media::{IORecordOptions, RecordOptions};
use yggdryl::{IOMedia, MediaType, MimeType};

#[test]
fn own_options_answer_the_handle_s_where_none_were_given() {
    let handle = Buffer::new().with_media_type(MediaType::new(MimeType::CSV));
    let own = own_options(&handle, None).expect("the handle's own");
    assert_eq!(own.mime_type(), MimeType::CSV);
    let given = RecordOptions::for_mime_type(&MimeType::ARROW_STREAM).expect("IPC options");
    let borrowed = own_options(&handle, Some(&given)).expect("the given ones");
    assert_eq!(borrowed.as_ref(), &given);
}

#[test]
fn dimension_options_drop_what_narrows_a_read() {
    let handle = Buffer::new().with_media_type(MediaType::new(MimeType::CSV));
    let narrowed = handle
        .record_options()
        .expect("options")
        .with_filter("id > 1")
        .expect("a filter")
        .with_max_row_size(3)
        .with_row_offset(1)
        .with_max_byte_size(2);
    let whole = dimension_options(
        &yggdryl::csv::Csv::new(handle).with_options(match narrowed {
            RecordOptions::Csv(csv) => csv,
            other => panic!("expected CSV options, got {other:?}"),
        }),
    )
    .expect("dimension options");
    assert_eq!(whole.filter().to_string(), "true");
    assert_eq!(whole.max_row_size(), None);
    assert_eq!(whole.row_offset(), None);
    assert_eq!(whole.max_byte_size(), None);
}
