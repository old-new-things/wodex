//! Opt-in LAN transfers. Native callbacks provide sockets and entropy; all HTTP,
//! authentication, queueing and lifetime policy stays in no_std Rust.
mod protocol;
#[cfg(test)]
mod tests;

use super::runtime::{JoinHandle, Mutex, Runtime};
use crate::state::{transfer::TransferChannel, MAX_SAVED_BYTES};
use alloc::{
  format,
  string::{String, ToString},
  sync::Arc,
  vec::Vec,
};
use core::sync::atomic::{AtomicBool, Ordering};
use protocol::{Method, Request, Response};

const HEADER_LIMIT: usize = 8192;
const HEADER_TIMEOUT_MS: u64 = 5000;
const BODY_TIMEOUT_MS: u64 = 30_000;
const WRITE_TIMEOUT_MS: u64 = 10_000;
const DRAIN_TIMEOUT_MS: u64 = 500;
const SESSION_TIMEOUT_MS: u64 = 15 * 60 * 1000;
const POLL_MS: u32 = 100;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct TransferApi {
  pub local_ipv4: unsafe extern "C" fn(*mut u8) -> bool,
  pub random: unsafe extern "C" fn(*mut u8, usize) -> bool,
  pub listen: unsafe extern "C" fn(*const u8, *mut u16) -> i32,
  pub accept: unsafe extern "C" fn(i32) -> i32,
  pub receive: unsafe extern "C" fn(i32, *mut u8, usize) -> i32,
  pub send: unsafe extern "C" fn(i32, *const u8, usize) -> i32,
  pub wait: unsafe extern "C" fn(i32, bool, u32) -> i32,
  pub close: unsafe extern "C" fn(i32),
  pub now_ms: unsafe extern "C" fn() -> u64,
  pub shutdown_write: unsafe extern "C" fn(i32) -> bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TransferStatus {
  pub supported: bool,
  pub active: bool,
  pub url: String,
  pub code: String,
  pub message: String,
  pub pending: bool,
}

#[derive(Clone)]
pub struct TransferExports {
  pub config: String,
  pub sessions: String,
}

pub struct TransferImport {
  pub channel: TransferChannel,
  pub data: String,
}

struct Shared {
  status: TransferStatus,
  exports: Option<TransferExports>,
  incoming: Option<TransferImport>,
}

pub struct Transfer {
  runtime: Runtime,
  api: Option<TransferApi>,
  shared: Arc<Mutex<Shared>>,
  stop: Arc<AtomicBool>,
  worker: Option<JoinHandle>,
}

impl Transfer {
  /// # Safety
  /// A non-null API must point to a valid table. Callbacks must not unwind,
  /// retain buffers or block outside `wait`; each socket is owned by one worker.
  /// `now_ms` must be monotonic, and random must use cryptographic entropy.
  pub unsafe fn new(runtime: Runtime, api: *const TransferApi) -> Result<Self, String> {
    let api = unsafe { api.as_ref() }.copied();
    let status = TransferStatus {
      supported: api.is_some(),
      message: if api.is_some() {
        "PC transfer is off.".into()
      } else {
        "PC transfer is unavailable on this build.".into()
      },
      ..TransferStatus::default()
    };
    Ok(Self {
      runtime,
      api,
      shared: Arc::new(runtime.mutex(Shared { status, exports: None, incoming: None })?),
      stop: Arc::new(AtomicBool::new(true)),
      worker: None,
    })
  }

  pub fn start(&mut self, exports: TransferExports) -> Result<(), String> {
    if self.active() {
      return Err("PC transfer is already running.".into());
    }
    self.stop();
    let api = self.api.ok_or("PC transfer is unavailable on this build.")?;
    if exports.config.len() > MAX_SAVED_BYTES || exports.sessions.len() > MAX_SAVED_BYTES {
      return Err("The transfer exceeds the 4 MiB document limit.".into());
    }
    let mut address = [0u8; 4];
    if !unsafe { (api.local_ipv4)(address.as_mut_ptr()) } || address[0] == 0 || address[0] >= 224 {
      return Err("Connect to Wi-Fi or Ethernet before starting PC transfer.".into());
    }
    let mut entropy = [0u8; 16];
    if !unsafe { (api.random)(entropy.as_mut_ptr(), entropy.len()) } {
      return Err("Cannot generate a secure pairing code.".into());
    }
    let code: String = entropy.iter().map(|byte| format!("{byte:02x}")).collect();
    entropy.fill(0);
    let mut port = 0;
    let fd = unsafe { (api.listen)(address.as_ptr(), &mut port) };
    if fd < 0 {
      return Err("Cannot open PC transfer on this network.".into());
    }
    let listener = Socket { api, fd };
    if port == 0 {
      return Err("The PC transfer listener returned an invalid port.".into());
    }
    let authority = format!("{}.{}.{}.{}:{port}", address[0], address[1], address[2], address[3]);
    let url = format!("http://{authority}");
    {
      let mut shared = self.shared.lock();
      shared.status = TransferStatus {
        supported: true,
        active: true,
        url,
        code: code.clone(),
        message: "Open this address on your PC. Transfer stops after 15 minutes.".into(),
        pending: false,
      };
      shared.exports = Some(exports);
    }
    self.stop.store(false, Ordering::Release);
    let shared = Arc::clone(&self.shared);
    let stop = Arc::clone(&self.stop);
    match self.runtime.spawn(move || serve(listener, authority, code, shared, stop)) {
      Ok(worker) => self.worker = Some(worker),
      Err(error) => {
        self.stop();
        self.shared.lock().status.message = error.clone();
        return Err(error);
      }
    }
    Ok(())
  }

  pub fn stop(&mut self) {
    self.stop.store(true, Ordering::Release);
    self.worker.take();
    let mut shared = self.shared.lock();
    clear_transfers(&mut shared);
    shared.status.active = false;
    shared.status.pending = false;
    shared.status.url.clear();
    wipe(&mut shared.status.code);
    if shared.status.supported {
      shared.status.message = "PC transfer is off.".into();
    }
  }

  pub fn status(&self) -> TransferStatus {
    self.shared.lock().status.clone()
  }

  pub fn active(&self) -> bool {
    self.shared.lock().status.active
  }

  pub fn take_import(&self) -> Option<TransferImport> {
    self.shared.lock().incoming.take()
  }

  pub fn resolve_import(&self, message: String, exports: Option<TransferExports>) {
    let mut shared = self.shared.lock();
    if !shared.status.active || !shared.status.pending {
      return;
    }
    if let Some(exports) = exports {
      if exports.config.len() > MAX_SAVED_BYTES || exports.sessions.len() > MAX_SAVED_BYTES {
        shared.status.message =
          "Cannot refresh the exported documents: size limit exceeded.".into();
        shared.status.pending = false;
        return;
      }
      clear_exports(&mut shared);
      shared.exports = Some(exports);
    }
    shared.incoming = None;
    shared.status.pending = false;
    shared.status.message = message;
  }
}

impl Drop for Transfer {
  fn drop(&mut self) {
    self.stop();
  }
}

fn wipe(text: &mut String) {
  // These transient copies contain pairing codes and may include the saved API
  // key. Do not leave their contents in a reusable allocation after stopping.
  for byte in unsafe { text.as_bytes_mut() } {
    unsafe { core::ptr::write_volatile(byte, 0) };
  }
  text.clear();
}

fn clear_exports(shared: &mut Shared) {
  if let Some(mut exports) = shared.exports.take() {
    wipe(&mut exports.config);
    wipe(&mut exports.sessions);
  }
}

fn clear_transfers(shared: &mut Shared) {
  clear_exports(shared);
  if let Some(mut incoming) = shared.incoming.take() {
    wipe(&mut incoming.data);
  }
}

struct Socket {
  api: TransferApi,
  fd: i32,
}

impl Drop for Socket {
  fn drop(&mut self) {
    unsafe { (self.api.close)(self.fd) };
  }
}

impl Socket {
  fn now(&self) -> u64 {
    unsafe { (self.api.now_ms)() }
  }

  fn ready(&self, writable: bool, deadline: u64, stop: &AtomicBool) -> bool {
    loop {
      if stop.load(Ordering::Acquire) {
        return false;
      }
      let remaining = deadline.saturating_sub(self.now());
      if remaining == 0 {
        return false;
      }
      match unsafe { (self.api.wait)(self.fd, writable, remaining.min(POLL_MS as u64) as u32) } {
        1 => return true,
        0 => {}
        _ => return false,
      }
    }
  }

  fn receive(&self, output: &mut [u8], deadline: u64, stop: &AtomicBool) -> Option<usize> {
    loop {
      if !self.ready(false, deadline, stop) {
        return None;
      }
      match unsafe { (self.api.receive)(self.fd, output.as_mut_ptr(), output.len()) } {
        -2 => {}
        count if count > 0 && count as usize <= output.len() => return Some(count as usize),
        _ => return None,
      }
    }
  }

  fn write(&self, mut bytes: &[u8], deadline: u64, stop: &AtomicBool) -> bool {
    while !bytes.is_empty() {
      if !self.ready(true, deadline, stop) {
        return false;
      }
      match unsafe { (self.api.send)(self.fd, bytes.as_ptr(), bytes.len()) } {
        -2 => {}
        count if count > 0 && count as usize <= bytes.len() => bytes = &bytes[count as usize..],
        _ => return false,
      }
    }
    true
  }

  fn respond(&self, response: Response<'_>, stop: &AtomicBool, session_deadline: u64) {
    let deadline = self.now().saturating_add(WRITE_TIMEOUT_MS).min(session_deadline);
    if !self.write(response.header().as_bytes(), deadline, stop)
      || !self.write(response.body, deadline, stop)
      || !unsafe { (self.api.shutdown_write)(self.fd) }
    {
      return;
    }
    // A rejected POST can still be arriving. Closing a socket with unread
    // incoming bytes sends RST and can discard the useful HTTP error response.
    // Half-close first so the peer receives the full response and FIN, then
    // drain without retaining data. Both time and bytes are capped, including
    // for unauthenticated callers, and stop remains responsive between polls.
    let deadline = self.now().saturating_add(DRAIN_TIMEOUT_MS).min(session_deadline);
    let mut remaining = MAX_SAVED_BYTES + HEADER_LIMIT;
    let mut bytes = [0u8; 4096];
    while remaining != 0 {
      let size = bytes.len().min(remaining);
      let Some(count) = self.receive(&mut bytes[..size], deadline, stop) else { break };
      remaining -= count;
    }
  }
}

fn serve(
  listener: Socket,
  authority: String,
  mut code: String,
  shared: Arc<Mutex<Shared>>,
  stop: Arc<AtomicBool>,
) {
  let deadline = listener.now().saturating_add(SESSION_TIMEOUT_MS);
  let mut failure = false;
  while listener.ready(false, deadline, &stop) {
    match unsafe { (listener.api.accept)(listener.fd) } {
      -2 => {}
      fd if fd >= 0 => {
        let client = Socket { api: listener.api, fd };
        handle_request(&client, &authority, &code, &shared, &stop, deadline);
      }
      _ => {
        failure = true;
        break;
      }
    }
  }
  wipe(&mut code);
  let mut shared = shared.lock();
  clear_transfers(&mut shared);
  shared.status.active = false;
  shared.status.pending = false;
  shared.status.url.clear();
  wipe(&mut shared.status.code);
  if !stop.load(Ordering::Acquire) {
    shared.status.message = if failure || listener.now() < deadline {
      "PC transfer stopped because the network connection closed.".into()
    } else {
      "PC transfer expired after 15 minutes. Start it again to get a new pairing code.".into()
    };
  }
}

fn handle_request(
  client: &Socket,
  authority: &str,
  code: &str,
  shared: &Mutex<Shared>,
  stop: &AtomicBool,
  session_deadline: u64,
) {
  let header_deadline = client.now().saturating_add(HEADER_TIMEOUT_MS).min(session_deadline);
  let mut bytes = Vec::new();
  let mut chunk = [0u8; 4096];
  let header_end = loop {
    if let Some(end) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
      break end + 4;
    }
    if bytes.len() >= HEADER_LIMIT {
      client.respond(Response::error(431), stop, session_deadline);
      return;
    }
    let size = chunk.len().min(HEADER_LIMIT - bytes.len());
    let Some(count) = client.receive(&mut chunk[..size], header_deadline, stop) else { return };
    bytes.extend_from_slice(&chunk[..count]);
  };
  let mut body = bytes.split_off(header_end);
  let request = match Request::parse(&bytes, authority, code) {
    Ok(request) => request,
    Err(status) => {
      client.respond(Response::error(status), stop, session_deadline);
      return;
    }
  };
  if body.len() > request.content_length {
    client.respond(Response::error(400), stop, session_deadline);
    return;
  }
  if request.method == Method::Post && shared.lock().status.pending {
    client.respond(Response::error(409), stop, session_deadline);
    return;
  }
  let body_deadline = client.now().saturating_add(BODY_TIMEOUT_MS).min(session_deadline);
  while body.len() < request.content_length {
    let size = chunk.len().min(request.content_length - body.len());
    let Some(count) = client.receive(&mut chunk[..size], body_deadline, stop) else { return };
    body.extend_from_slice(&chunk[..count]);
  }
  if stop.load(Ordering::Acquire) || client.now() >= session_deadline {
    return;
  }
  match (request.method, request.path) {
    (Method::Get, "/") => client.respond(
      Response::asset(
        "text/html; charset=utf-8",
        include_bytes!("../../../romfs/transfer/index.html"),
      ),
      stop,
      session_deadline,
    ),
    (Method::Get, "/transfer.js") => client.respond(
      Response::asset(
        "text/javascript; charset=utf-8",
        include_bytes!("../../../romfs/transfer/transfer.js"),
      ),
      stop,
      session_deadline,
    ),
    (Method::Get, "/transfer.css") => client.respond(
      Response::asset(
        "text/css; charset=utf-8",
        include_bytes!("../../../romfs/transfer/transfer.css"),
      ),
      stop,
      session_deadline,
    ),
    (Method::Get, "/api/status") => {
      let status = status_json(&shared.lock());
      client.respond(Response::json(200, status.as_bytes()), stop, session_deadline);
    }
    (Method::Get, path @ ("/api/config" | "/api/sessions")) => {
      let mut document = {
        let shared = shared.lock();
        let Some(exports) = &shared.exports else { return };
        if path == "/api/config" {
          exports.config.clone()
        } else {
          exports.sessions.clone()
        }
      };
      client.respond(
        Response::download(document.as_bytes(), path == "/api/config"),
        stop,
        session_deadline,
      );
      wipe(&mut document);
    }
    (Method::Post, "/api/config" | "/api/sessions") => {
      let data = match String::from_utf8(body) {
        Ok(data) => data,
        Err(_) => {
          client.respond(Response::error(400), stop, session_deadline);
          return;
        }
      };
      let status = {
        let mut shared = shared.lock();
        shared.incoming = Some(TransferImport {
          channel: if request.path == "/api/config" {
            TransferChannel::Config
          } else {
            TransferChannel::Sessions
          },
          data,
        });
        shared.status.pending = true;
        shared.status.message = "Confirm this import on your Switch.".into();
        status_json(&shared)
      };
      client.respond(Response::json(202, status.as_bytes()), stop, session_deadline);
    }
    _ => client.respond(Response::error(404), stop, session_deadline),
  }
}

fn status_json(shared: &Shared) -> String {
  serde_json::json!({
    "active": shared.status.active,
    "pending": shared.status.pending,
    "message": shared.status.message,
  })
  .to_string()
}
