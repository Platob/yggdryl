//! One test file per file under `rust/src/xmla/`, under `tests/xmla/`.
//!
//! `rust/tests/` mirrors `rust/src/`: a source file has exactly one test file
//! at the matching path, and this target is the harness for the XML for
//! Analysis medium, its provider and its server. A test reaches the crate
//! through `yggdryl::` and nothing else.

#[path = "xmla/catalog.rs"]
mod catalog;
#[path = "xmla/dbtype.rs"]
mod dbtype;
#[path = "xmla/definitions.rs"]
mod definitions;
#[path = "xmla/media.rs"]
mod media;
#[path = "xmla/mod_.rs"]
mod mod_;
#[path = "xmla/options.rs"]
mod options;
#[path = "xmla/request.rs"]
mod request;
#[path = "xmla/response.rs"]
mod response;
#[path = "xmla/rowset.rs"]
mod rowset;
#[path = "xmla/server.rs"]
mod server;
#[path = "xmla/service.rs"]
mod service;
#[path = "xmla/vocabulary.rs"]
mod vocabulary;
