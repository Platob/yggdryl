//! One test file per file under `xmla/src/`, under `tests/root/`.
//!
//! `xmla/tests/` mirrors `xmla/src/`: a source file has exactly one test file
//! at the matching path, and this target is the harness for the XML for
//! Analysis medium, its provider and its server. A test reaches the crate
//! through `yggdryl_xmla::` and the core through `yggdryl::`, nothing else.

#[path = "root/dbtype.rs"]
mod dbtype;
#[path = "root/definitions.rs"]
mod definitions;
#[path = "root/lib.rs"]
mod lib;
#[path = "root/media.rs"]
mod media;
#[path = "root/options.rs"]
mod options;
#[path = "root/request.rs"]
mod request;
#[path = "root/response.rs"]
mod response;
#[path = "root/rowset.rs"]
mod rowset;
#[cfg(feature = "http")]
#[path = "root/server.rs"]
mod server;
#[path = "root/service.rs"]
mod service;
#[path = "root/vocabulary.rs"]
mod vocabulary;
