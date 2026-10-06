//! `rust/src/pluginside.rs`: the role of a FIX plugin - Buy-Side, Sell-Side
//! or none stated - as one enum leaf stored as a `uint8`.

use std::sync::Arc;

use arrow_array::{Array, ArrayRef, StringArray, UInt8Array};
use arrow_schema::DataType as ArrowDataType;
use yggdryl::{
    ArrowCastOptions, ArrowCastPlan, DataType, DataTypeId, DataTypeKind, Field, PluginSide, Scalar,
    Serie, Side, State,
};

#[test]
fn the_plugin_side_is_an_enum_leaf_over_uint8_codes() {
    let dtype = DataType::PluginSide;
    assert_eq!(DataType::from_str("pluginside").unwrap(), dtype);
    assert_eq!(DataType::from_logical_name("pluginside").unwrap(), dtype);
    assert_eq!(DataType::pluginside(), dtype);
    assert_eq!(dtype.to_string(), "pluginside");
    assert_eq!(dtype.kind(), DataTypeKind::Enum);
    assert!(dtype.is_enum() && !dtype.is_code());
    assert_eq!(dtype.code_width(), None);
    assert_eq!(dtype.id(), DataTypeId::PluginSide);
    assert_eq!(dtype.id().as_u8(), 0xc6);
    assert_eq!(
        DataTypeId::from_str("pluginside").unwrap(),
        DataTypeId::PluginSide
    );

    let field = Field::new("msgpluginside", dtype.clone(), false);
    let arrow = field.clone().into_arrow_field().unwrap();
    assert_eq!(arrow.data_type(), &ArrowDataType::UInt8);
    assert_eq!(
        arrow.metadata()["ARROW:extension:name"],
        "yggdryl.pluginside"
    );
    assert_eq!(Field::from_arrow_field(&arrow).unwrap(), field);
    assert_eq!(std::mem::size_of::<PluginSide>(), 1);
    // The schema document round trips under the one tag.
    let json = dtype.clone().into_json().unwrap();
    assert!(json.contains("pluginside"), "{json}");
    assert_eq!(DataType::from_json(&json).unwrap(), dtype);
}

/// Three members in code order, the neutral one first: a stored name, a
/// code and a description each, and the default is the neutral member.
#[test]
fn the_members_are_the_two_roles_beside_none_stated() {
    assert_eq!(
        PluginSide::ALL,
        [
            PluginSide::Unknown,
            PluginSide::BuySide,
            PluginSide::SellSide
        ]
    );
    for (member, code, name) in [
        (PluginSide::Unknown, 0, "UKNW"),
        (PluginSide::BuySide, 1, "BUYS"),
        (PluginSide::SellSide, 2, "SELL"),
    ] {
        assert_eq!(member.code(), code, "{name}");
        assert_eq!(member.as_str(), name);
        assert_eq!(member.to_string(), name);
        assert_eq!(PluginSide::from_code(code), Some(member));
        assert_eq!(PluginSide::from_name(name), Some(member));
        assert_eq!(PluginSide::read_code(i64::from(code)).unwrap(), member);
        assert_eq!(u8::from(member), code);
        assert_eq!(PluginSide::try_from(code).unwrap(), member);
        assert!(!member.description().is_empty());
    }
    assert_eq!(PluginSide::default(), PluginSide::Unknown);
    assert!(PluginSide::BuySide.description().contains("Buy-Side"));
    assert!(PluginSide::SellSide.description().contains("Sell-Side"));
    assert_eq!(PluginSide::from_code(3), None);
    assert!(PluginSide::read_code(3).is_err());
    assert!(PluginSide::read_code(-1).is_err());
}

/// The stored name in any case and the role's own name folded reach one
/// member; a spelling naming none is refused, naming it.
#[test]
fn a_spelling_reads_as_one_member_and_a_stranger_is_refused() {
    for (spelling, member) in [
        ("BUYS", PluginSide::BuySide),
        ("buys", PluginSide::BuySide),
        ("BuySide", PluginSide::BuySide),
        ("buyside", PluginSide::BuySide),
        ("buy-side", PluginSide::BuySide),
        ("buy_side", PluginSide::BuySide),
        ("BUY SIDE", PluginSide::BuySide),
        ("SELL", PluginSide::SellSide),
        ("sell", PluginSide::SellSide),
        ("SellSide", PluginSide::SellSide),
        ("sell-side", PluginSide::SellSide),
        ("sell_side", PluginSide::SellSide),
        ("Sell Side", PluginSide::SellSide),
        ("UKNW", PluginSide::Unknown),
        ("uknw", PluginSide::Unknown),
        ("Unknown", PluginSide::Unknown),
    ] {
        assert_eq!(
            PluginSide::from_spelling(spelling),
            Some(member),
            "{spelling}"
        );
        assert_eq!(PluginSide::read(spelling).unwrap(), member, "{spelling}");
        assert_eq!(
            serde_json::from_str::<PluginSide>(&format!("{spelling:?}")).unwrap(),
            member,
            "{spelling}"
        );
    }
    for stranger in ["", "X", "1", "ask", "UNKN", "middle-side"] {
        assert_eq!(PluginSide::from_spelling(stranger), None, "{stranger:?}");
        let refused = PluginSide::read(stranger).unwrap_err().to_string();
        assert!(refused.contains("pluginside"), "{refused}");
        assert!(refused.contains(&format!("{stranger:?}")), "{refused}");
        assert!(serde_json::from_str::<PluginSide>(&format!("{stranger:?}")).is_err());
    }
    // Serialized as the stored name, read back from it or from the code.
    assert_eq!(
        serde_json::to_string(&PluginSide::SellSide).unwrap(),
        "\"SELL\""
    );
    assert_eq!(
        serde_json::from_str::<PluginSide>("2").unwrap(),
        PluginSide::SellSide
    );

    // The value door reads the same spellings and codes: text is a
    // spelling, an integer a code.
    assert_eq!(
        DataType::PluginSide.scalar("sell-side").unwrap(),
        Scalar::PluginSide(PluginSide::SellSide)
    );
    assert_eq!(
        DataType::PluginSide.scalar(1_i32).unwrap(),
        Scalar::PluginSide(PluginSide::BuySide)
    );
    assert_eq!(
        DataType::PluginSide
            .scalar(Scalar::PluginSide(PluginSide::Unknown))
            .unwrap(),
        Scalar::PluginSide(PluginSide::Unknown)
    );
    assert!(DataType::PluginSide.scalar("X").is_err());
    assert!(DataType::PluginSide.scalar(3_i32).is_err());
    assert!(DataType::PluginSide.scalar(Scalar::from("UNKN")).is_err());
}

/// A CBlock's root `type` names the plugin's class, and the role is read
/// off its last segment alone, folded as every name folds - case, `_`, `-`
/// and blanks dropped: `BuySide` in it is `BUYS`, `SellSide` is `SELL`, and
/// anything else - a package naming a side, a class naming neither, no
/// attribute - is `UKNW`, never a refusal.
#[test]
fn from_plugin_type_reads_the_role_off_the_last_segment_of_the_class() {
    for (class, member) in [
        (
            "com.ullink.ulbridge2.toolkit.plugins.fix.model.state.cblock.BuySideFIXCPluginCBlock",
            PluginSide::BuySide,
        ),
        (
            "com.ullink.ulbridge2.toolkit.plugins.fix.model.state.cblock.SellSideFIXCPluginCBlock",
            PluginSide::SellSide,
        ),
        ("BuySideFIXCPluginCBlock", PluginSide::BuySide),
        ("SELLSIDEFIXCPLUGINCBLOCK", PluginSide::SellSide),
        ("a.b.sellside", PluginSide::SellSide),
        ("x.MyBuySidePlugin", PluginSide::BuySide),
        // The fold every name in the crate reads by, not a lowercase alone.
        ("x.Buy_Side_FIXCPluginCBlock", PluginSide::BuySide),
        ("Sell-Side FIXCPluginCBlock", PluginSide::SellSide),
        ("BUY SIDE", PluginSide::BuySide),
        // Only the last segment names the role.
        ("buyside.FIXCPluginCBlock", PluginSide::Unknown),
        ("com.x.cblock.FIXCPluginCBlock", PluginSide::Unknown),
        ("", PluginSide::Unknown),
        ("buy.side", PluginSide::Unknown),
    ] {
        assert_eq!(PluginSide::from_plugin_type(class), member, "{class:?}");
    }
}

/// `BUYS` and `SELL` are spelled like two sides of a trade and are not
/// them: a plugin's role is its own enum, and neither value door takes the
/// other's member.
#[test]
fn a_plugin_side_is_never_a_side() {
    assert_ne!(
        Scalar::PluginSide(PluginSide::BuySide),
        Scalar::Side(Side::Buy)
    );
    assert_ne!(DataTypeId::PluginSide, DataTypeId::Side);
    assert_eq!(
        Scalar::PluginSide(PluginSide::SellSide).kind(),
        "pluginside"
    );
    assert_eq!(Scalar::Side(Side::Sell).kind(), "side");
    assert!(
        DataType::PluginSide
            .scalar(Scalar::Side(Side::Buy))
            .is_err(),
        "a member of another enum leaf is refused"
    );
    assert!(
        DataType::Side
            .scalar(Scalar::PluginSide(PluginSide::SellSide))
            .is_err()
    );
}

/// The column cast keeps the two enums apart as the value door does:
/// `Side` stores `BUYS` as `1` and `SELL` as `2`, the codes `PluginSide`
/// stores its two roles under, and the engine never reads one leaf's codes
/// as the other's - a side column cast into plugin roles is refused by
/// name, and so is the reverse, whatever `safe` says and whether the cast
/// is compiled once or run on the column. The rule is the enum family's,
/// not the pair's: a state column is refused the same way. A text column
/// spelling the names still lands through the spelling reader.
#[test]
fn a_column_of_one_enum_leaf_is_never_cast_into_another() {
    let sides = Serie::from_scalars(
        Field::new("s", DataType::Side, false),
        [Scalar::Side(Side::Buy), Scalar::Side(Side::Sell)],
    )
    .unwrap();
    let roles = Serie::from_scalars(
        Field::new("p", DataType::PluginSide, false),
        [
            Scalar::PluginSide(PluginSide::BuySide),
            Scalar::PluginSide(PluginSide::SellSide),
        ],
    )
    .unwrap();
    let states = Serie::from_scalars(
        Field::new("st", DataType::State, false),
        [Scalar::State(State::New), Scalar::State(State::Filled)],
    )
    .unwrap();
    for (source, target) in [
        (&sides, DataType::PluginSide),
        (&roles, DataType::Side),
        (&states, DataType::PluginSide),
        (&roles, DataType::State),
    ] {
        let target = Field::new("t", target, false);
        for safe in [true, false] {
            let refusal = source
                .cast(&target, ArrowCastOptions::new().with_safe(safe))
                .unwrap_err()
                .to_string();
            assert!(
                refusal.contains(&source.field().unwrap().dtype().to_string())
                    && refusal.contains(&target.dtype().to_string()),
                "{refusal}"
            );
            assert!(
                ArrowCastPlan::compile(
                    source.field().unwrap(),
                    &target,
                    ArrowCastOptions::new().with_safe(safe)
                )
                .is_err(),
                "a plan onto {} from {}",
                target.dtype(),
                source.field().unwrap().dtype()
            );
        }
    }
    // Its own leaf passes untouched, and an integer reads the codes.
    let same = roles
        .cast(
            &Field::new("same", DataType::PluginSide, false),
            ArrowCastOptions::new(),
        )
        .unwrap();
    assert_eq!(
        same.scalar(1).unwrap(),
        Scalar::PluginSide(PluginSide::SellSide)
    );
    let codes = roles
        .cast(
            &Field::new("codes", DataType::Int32, false),
            ArrowCastOptions::new(),
        )
        .unwrap();
    assert_eq!(codes.scalar(1).unwrap(), Scalar::from(2_i32));
}

/// A text column lands as the codes its spellings name, the column reads
/// back member by member, and it renders as the stored names.
#[test]
fn a_text_column_lands_as_codes_and_renders_as_names() {
    let field = Field::new("msgpluginside", DataType::PluginSide, false);
    let landed = Serie::from_arrow_array(
        Some(&field),
        Arc::new(StringArray::from(vec!["buyside", "SELL", "UKNW"])) as ArrayRef,
        ArrowCastOptions::new().with_safe(false),
    )
    .unwrap();
    let codes = landed.require_arrow_array().unwrap();
    let codes = codes.as_any().downcast_ref::<UInt8Array>().unwrap();
    assert_eq!(codes.values().as_ref(), [1, 2, 0]);
    assert_eq!(
        landed.scalar(0).unwrap(),
        Scalar::PluginSide(PluginSide::BuySide)
    );
    assert_eq!(
        landed.scalar(2).unwrap(),
        Scalar::PluginSide(PluginSide::Unknown)
    );
    let names = landed
        .cast(
            &Field::new("x", DataType::utf8(), true),
            ArrowCastOptions::new().with_safe(false),
        )
        .unwrap();
    assert_eq!(names.scalar(1).unwrap(), Scalar::from("SELL"));

    // Rows lay out the same way and come back as the members they are.
    let rows = Serie::from_scalars(
        field,
        [
            Scalar::PluginSide(PluginSide::SellSide),
            Scalar::from("buy-side"),
            Scalar::from(0_i32),
        ],
    )
    .unwrap();
    assert_eq!(rows.len(), 3);
    assert_eq!(
        (0..3)
            .map(|at| rows.scalar(at).unwrap())
            .collect::<Vec<_>>(),
        [
            Scalar::PluginSide(PluginSide::SellSide),
            Scalar::PluginSide(PluginSide::BuySide),
            Scalar::PluginSide(PluginSide::Unknown),
        ]
    );
    // A spelling naming no member is refused where the rows land.
    assert!(
        Serie::from_scalars(
            Field::new("msgpluginside", DataType::PluginSide, false),
            [Scalar::from("UNKN")],
        )
        .is_err()
    );
}
