//! An installed default wins over every other resolution step.
//!
//! The process default is process-wide, so the layer binary runs this case in
//! its own selected child process with a default nothing else touched.

use std::sync::Arc;

use yggdryl::{DataType, FixMsg, FixRegistry, Scalar};

#[test]
fn an_installed_registry_is_the_default_and_cannot_be_replaced() {
    if super::run_isolated(
        "global_install::an_installed_registry_is_the_default_and_cannot_be_replaced",
        "install",
    ) {
        return;
    }
    let mut symbol = DataType::Utf8.required_field("Symbol");
    symbol.as_fix_mut().set_tag(55).expect("a valid tag");
    let registry = FixRegistry::from_fields([symbol.clone()]).expect("one field");
    FixRegistry::install_global(registry).expect("nothing has resolved the default yet");

    let global = FixRegistry::global().expect("the installed registry");
    assert_eq!(global.field_by_tag(55).expect("Symbol").name(), "Symbol");

    // A message built without a registry links that very `Arc`.
    let root = DataType::from_fields([symbol])
        .expect("one child")
        .required_field("row");
    let value = Scalar::from_record([("Symbol", Scalar::from("AAPL"))]).expect("one entry");
    let msg = FixMsg::new(root, value).expect("a valid message");
    assert!(Arc::ptr_eq(msg.registry(), global));
    assert_eq!(msg.by_tag(55).expect("Symbol"), &Scalar::from("AAPL"));

    // Once resolved, the default cannot change underneath its callers.
    let error = FixRegistry::install_global(FixRegistry::new()).expect_err("already resolved");
    assert!(error.is_conflict(), "{error}");
    assert!(error.to_string().contains("already resolved"), "{error}");
    assert!(Arc::ptr_eq(
        FixRegistry::global().expect("the same registry"),
        global
    ));
}
