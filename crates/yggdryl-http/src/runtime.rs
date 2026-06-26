//! A shared `tokio` runtime for the async HTTP/2 (and HTTP/3) transport layer.
//!
//! The runtime is initialised lazily the first time an H2/H3 request is made and
//! lives for the process lifetime (a `tokio::runtime::Runtime` cannot be cheaply
//! recreated). Every blocking entry point drives the async transport by calling
//! [`block_on`], which polls the given future on the calling thread using the
//! shared runtime's scheduler — the future does **not** need to be `Send`.

static RT: std::sync::OnceLock<tokio::runtime::Runtime> = std::sync::OnceLock::new();

/// Returns the shared multi-thread `tokio` runtime, initialising it on first call.
pub(crate) fn runtime() -> &'static tokio::runtime::Runtime {
    RT.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("failed to build async HTTP/2+HTTP/3 runtime")
    })
}

/// Drives `future` to completion on the calling thread, using the shared runtime's
/// I/O reactor and thread pool for any sub-tasks spawned inside it.
pub(crate) fn block_on<F: std::future::Future>(future: F) -> F::Output {
    runtime().block_on(future)
}
