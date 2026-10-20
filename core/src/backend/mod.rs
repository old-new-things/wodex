//! Rust-owned I/O backends. Native tables expose ABI-specific primitives only.
mod ffi;
pub(crate) mod http;
pub(crate) mod images;
pub(crate) mod runtime;
pub(crate) mod storage;
pub(crate) mod transport;
pub(crate) mod updater;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct BackendApi {
  pub fs: storage::FsApi,
  pub runtime: runtime::RuntimeApi,
  pub http: http::HttpApi,
}

pub struct PendingHandle {
  pub value: updater::PendingUpdate,
}
