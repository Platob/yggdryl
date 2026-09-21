//! Record-format integration tests, one module per media family.

#[path = "media/inference.rs"]
mod inference;
#[path = "media/magic.rs"]
mod magic;
#[path = "media/merge.rs"]
mod merge;
#[path = "media/mod_.rs"]
mod mod_;
#[path = "media/options.rs"]
mod options;
#[path = "media/partition.rs"]
mod partition;
#[path = "media/structured.rs"]
mod structured;
