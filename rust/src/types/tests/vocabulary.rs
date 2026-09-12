use super::super::DataType;
use crate::StringEnum;

fn lists() -> [(&'static str, &'static [&'static str]); 3] {
    [
        ("currency", StringEnum::CURRENCIES),
        ("country", StringEnum::COUNTRIES),
        ("mic", StringEnum::MICS),
    ]
}

#[test]
fn every_constant_is_sorted_unique_and_fits_its_width() {
    for (name, values) in lists() {
        assert!(!values.is_empty(), "{name} prebuilds nothing");
        assert!(
            values.windows(2).all(|pair| pair[0] < pair[1]),
            "{name} is not sorted and deduplicated"
        );
        let width = DataType::from_logical_name(name)
            .unwrap()
            .fixed_byte_width()
            .unwrap();
        for value in values {
            assert!(
                value.is_ascii() && !value.is_empty() && value.len() <= width,
                "{name} holds {value:?}, which does not fit {width} bytes"
            );
            assert!(
                value
                    .bytes()
                    .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit()),
                "{name} holds {value:?}, which is not an uppercase code"
            );
        }
    }
    // The code sets are the standards' own shapes.
    assert!(StringEnum::CURRENCIES.iter().all(|code| code.len() == 3));
    assert!(StringEnum::COUNTRIES.iter().all(|code| code.len() == 2));
    assert!(StringEnum::MICS.iter().all(|code| code.len() == 4));
}

/// An ISO code is already an identifier, so no two members collide and
/// the member is the code itself.
#[test]
fn every_constant_names_its_own_enum_members() {
    for (name, values) in lists() {
        let declared = StringEnum::from_logical_name(name).unwrap();
        let dtype = DataType::from_logical_name(name).unwrap();
        assert_eq!(declared.len(), values.len(), "{name}");
        assert_eq!(declared.name(), name, "{name}");
        for value in values {
            assert_eq!(declared.get(value), Some(*value), "{value}");
        }
        // A member is the integer its value packs into under the width
        // the name resolves to, never a position in this listing.
        let members = declared.into_members(&dtype).unwrap();
        for (member, code) in &members {
            assert_eq!(*code, dtype.ascii_packed(member.as_bytes()).unwrap());
        }
    }
}

#[test]
fn the_two_names_of_one_list_prebuild_one_vocabulary() {
    assert_eq!(
        StringEnum::from_logical_name("Exchange").unwrap().len(),
        StringEnum::from_logical_name("mic").unwrap().len()
    );
    assert_eq!(StringEnum::prebuilt_values(" MIC "), StringEnum::MICS);
    assert!(StringEnum::prebuilt_values("isin").is_empty());
}

#[test]
fn a_registered_name_with_no_constant_prebuilds_no_members() {
    for name in ["language", "monthyear", "tenor"] {
        assert!(
            StringEnum::from_logical_name(name).unwrap().is_empty(),
            "{name}"
        );
    }
}

#[test]
fn a_name_that_is_not_registered_is_refused_by_the_vocabulary() {
    let refused = StringEnum::from_logical_name("sedol")
        .unwrap_err()
        .to_string();
    assert!(refused.contains("currency"), "{refused}");
}
