//! Streaming chat transport, including validation, worker lifetime and queue bounds.
use super::{
  http::{Http, HttpError, Request},
  runtime::{JoinHandle, Mutex, Runtime},
};
use crate::app::host::TransportEvent;
use alloc::{
  collections::VecDeque,
  format,
  string::{String, ToString},
  sync::Arc,
  vec,
  vec::Vec,
};
use core::sync::atomic::{AtomicBool, Ordering};

const MAX_QUEUE_BYTES: usize = 8 * 1024 * 1024;
const CHUNK_SIZE: usize = 16 * 1024;
const MAX_REQUEST_BYTES: usize = 4 * 1024 * 1024;

#[derive(Default)]
struct Queue {
  events: VecDeque<TransportEvent>,
  bytes: usize,
}
struct Shared {
  queue: Mutex<Queue>,
  cancelled: AtomicBool,
  active: AtomicBool,
}

pub struct Transport {
  runtime: Runtime,
  http: Http,
  ca_bundle: String,
  shared: Arc<Shared>,
  worker: Option<JoinHandle>,
}

impl Transport {
  pub fn new(runtime: Runtime, http: Http, ca_bundle: String) -> Result<Self, String> {
    Ok(Self {
      runtime,
      http,
      ca_bundle,
      shared: Arc::new(Shared {
        queue: runtime.mutex(Queue::default())?,
        cancelled: AtomicBool::new(false),
        active: AtomicBool::new(false),
      }),
      worker: None,
    })
  }

  pub fn start(&mut self, url: &str, api_key: &str, body: &str) -> Result<(), String> {
    let invalid_key = api_key.is_empty() || api_key.len() > 4096 || has_control(api_key);
    let headers = if invalid_key {
      Vec::new()
    } else {
      vec![(String::from("Authorization"), format!("Bearer {api_key}"))]
    };
    self.start_with_headers(
      url,
      &headers,
      body,
      false,
      invalid_key.then_some("Enter a valid API key without control characters."),
    )
  }

  pub fn start_prepared(
    &mut self,
    url: &str,
    headers: &[(String, String)],
    body: &str,
    allow_insecure_http: bool,
  ) -> Result<(), String> {
    self.start_with_headers(url, headers, body, allow_insecure_http, None)
  }

  pub fn start_request(&mut self, request: &serde_json::Value) -> Result<(), String> {
    let (Some(url), Some(body)) = (request["url"].as_str(), request.get("body")) else {
      return Err("The provider request is incomplete.".into());
    };
    let headers: Vec<(String, String)> = serde_json::from_value(request["headers"].clone())
      .map_err(|_| String::from("The provider request headers are invalid."))?;
    let allow_insecure_http = request
      .get("allow_insecure_http")
      .map(|value| value.as_bool().ok_or("The local HTTP setting is invalid."))
      .transpose()?
      .unwrap_or(false);
    self.start_prepared(url, &headers, &body.to_string(), allow_insecure_http)
  }

  fn start_with_headers(
    &mut self,
    url: &str,
    headers: &[(String, String)],
    body: &str,
    allow_insecure_http: bool,
    validation_error: Option<&str>,
  ) -> Result<(), String> {
    if self.active() {
      return Err("A request is already in progress.".into());
    }
    if let Some(worker) = self.worker.take() {
      worker.join();
    }
    *self.shared.queue.lock() = Queue::default();
    self.shared.cancelled.store(false, Ordering::Release);
    self.shared.active.store(true, Ordering::Release);
    let error = if !valid_url(self.http, url, allow_insecure_http) {
      Some("Use HTTPS, or explicitly allow private LAN HTTP in llama.cpp settings.")
    } else if !valid_headers(headers) {
      Some("The provider prepared invalid HTTP headers.")
    } else if validation_error.is_some() {
      validation_error
    } else if body.is_empty() || body.len() > MAX_REQUEST_BYTES {
      Some("The request is empty or exceeds the 4 MiB limit.")
    } else {
      None
    };
    if let Some(error) = error {
      self.shared.finish(0, error.into());
      return Ok(());
    }
    let mut request = Request::default();
    request.url = url.into();
    request.post = true;
    request.headers = vec!["Expect:".into()];
    for (name, value) in headers {
      request.headers.push(format!("{name}: {value}"));
    }
    for (name, value) in [("Content-Type", "application/json"), ("Accept", "text/event-stream")] {
      if !headers.iter().any(|(key, _)| key.eq_ignore_ascii_case(name)) {
        request.headers.push(format!("{name}: {value}"));
      }
    }
    request.body = body.as_bytes().to_vec();
    request.ca_bundle = self.ca_bundle.clone();
    request.no_proxy = url.get(..7).is_some_and(|scheme| scheme.eq_ignore_ascii_case("http://"));
    request.protocols = "http,https".into();
    let shared = Arc::clone(&self.shared);
    let http = self.http;
    match self.runtime.spawn(move || {
      let mut sink = |bytes: &[u8]| shared.receive(bytes);
      let (status, error) = match http.perform(&request, &shared.cancelled, &mut sink) {
        Err(HttpError::Cancelled) => (0, "Request cancelled.".into()),
        Err(HttpError::Failure(error)) => (0, error),
        Ok(response) if response.curl_code != 0 => {
          (response.http_status, transfer_error(response.curl_code).into())
        }
        Ok(response) if !(200..300).contains(&response.http_status) => {
          (response.http_status, http_error(response.http_status))
        }
        Ok(response) => (response.http_status, String::new()),
      };
      shared.finish(status, error);
    }) {
      Ok(worker) => self.worker = Some(worker),
      Err(error) => self.shared.finish(0, error),
    }
    Ok(())
  }

  pub fn cancel(&self) {
    self.shared.cancelled.store(true, Ordering::Release);
  }
  pub fn active(&self) -> bool {
    self.shared.active.load(Ordering::Acquire)
  }
  pub fn poll(&mut self) -> Vec<TransportEvent> {
    let mut queue = self.shared.queue.lock();
    queue.bytes = 0;
    queue.events.drain(..).collect()
  }
}

impl Drop for Transport {
  fn drop(&mut self) {
    self.cancel();
    if let Some(worker) = self.worker.take() {
      worker.join();
    }
  }
}

impl Shared {
  fn finish(&self, http_status: u16, error: String) {
    let mut queue = self.queue.lock();
    queue.events.push_back(TransportEvent::Finished { http_status, error });
    // Observing Finished through this lock must also observe an idle worker.
    self.active.store(false, Ordering::Release);
  }
  fn receive(&self, mut bytes: &[u8]) -> Result<(), String> {
    if self.cancelled.load(Ordering::Acquire) {
      return Err("Request cancelled.".into());
    }
    let mut queue = self.queue.lock();
    if bytes.len() > MAX_QUEUE_BYTES - queue.bytes {
      return Err("The response exceeded the 8 MiB buffer limit.".into());
    }
    while !bytes.is_empty() {
      if !matches!(queue.events.back(), Some(TransportEvent::Data(chunk)) if chunk.len() < CHUNK_SIZE)
      {
        queue.events.push_back(TransportEvent::Data(Vec::new()));
      }
      let Some(TransportEvent::Data(chunk)) = queue.events.back_mut() else { unreachable!() };
      let count = bytes.len().min(CHUNK_SIZE - chunk.len());
      chunk.extend_from_slice(&bytes[..count]);
      queue.bytes += count;
      bytes = &bytes[count..];
    }
    Ok(())
  }
}

fn has_control(text: &str) -> bool {
  text.bytes().any(|byte| byte < 0x20 || byte == 0x7f)
}
fn valid_headers(headers: &[(String, String)]) -> bool {
  headers.len() <= 16
    && headers.iter().enumerate().all(|(index, (name, value))| {
      ["authorization", "x-api-key", "content-type", "accept", "anthropic-version", "user-agent"]
        .iter()
        .any(|allowed| name.eq_ignore_ascii_case(allowed))
        && !value.is_empty()
        && value.len() <= 8192
        && !has_control(value)
        && !headers[..index].iter().any(|(earlier, _)| earlier.eq_ignore_ascii_case(name))
    })
}
fn valid_url(http: Http, text: &str, allow_lan_http: bool) -> bool {
  if crate::api_url::validate_request(text, allow_lan_http).is_err() {
    return false;
  }
  let Ok(url) = http.parse_url(text) else {
    return false;
  };
  if url.host.is_empty() || url.has_user || url.has_password || url.has_fragment {
    return false;
  }
  url.scheme == "https"
    || (url.scheme == "http"
      && (matches!(url.host.as_str(), "localhost" | "127.0.0.1" | "[::1]" | "::1")
        || (allow_lan_http && crate::api_url::is_lan_host(&url.host))))
}

fn transfer_error(code: u32) -> &'static str {
  match code {
    6 => "Cannot resolve the API server. Check the endpoint and network.",
    5 => "Cannot resolve the configured network proxy.",
    7 => "Cannot connect to the API server.",
    28 => "The request timed out. Please try again.",
    60 => "The API server's TLS certificate could not be verified.",
    77 => "Cannot read the trusted TLS certificate bundle.",
    35 => "A secure connection to the API server could not be established.",
    _ => "The network request failed. Check your connection and try again.",
  }
}
// Error bodies may contain echoed secrets. Only status-specific guidance is shown.
fn http_error(status: u16) -> String {
  match status {
    401 | 403 => "API access denied. Check your API key and account permissions.".into(),
    402 => "The API account has insufficient balance.".into(),
    429 => "The API rate limit was reached. Wait a moment and try again.".into(),
    300..=399 => "The API returned a redirect. Enter its final HTTPS endpoint in Settings.".into(),
    500.. => "The API server is temporarily unavailable. Please try again.".into(),
    _ => format!("The API rejected the request (HTTP {status}). Check your model and endpoint."),
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  fn shared() -> Shared {
    Shared {
      queue: super::super::runtime::tests::runtime().mutex(Queue::default()).unwrap(),
      cancelled: AtomicBool::new(false),
      active: AtomicBool::new(true),
    }
  }

  #[test]
  fn queue_merges_chunks_but_stays_bounded_and_preserves_completion_order() {
    let shared = shared();
    shared.receive(&[b'a'; 5]).unwrap();
    shared.receive(&vec![b'b'; CHUNK_SIZE * 2]).unwrap();
    {
      let queue = shared.queue.lock();
      let chunks: Vec<_> = queue
        .events
        .iter()
        .map(|event| match event {
          TransportEvent::Data(bytes) => bytes.len(),
          _ => 0,
        })
        .collect();
      assert_eq!(chunks, vec![CHUNK_SIZE, CHUNK_SIZE, 5]);
      assert_eq!(queue.bytes, CHUNK_SIZE * 2 + 5);
    }
    let too_large = vec![0; MAX_QUEUE_BYTES];
    assert!(shared.receive(&too_large).unwrap_err().contains("8 MiB"));
    assert_eq!(shared.queue.lock().bytes, CHUNK_SIZE * 2 + 5);
    shared.finish(200, String::new());
    let queue = shared.queue.lock();
    assert!(matches!(queue.events.back(), Some(TransportEvent::Finished { http_status: 200, .. })));
    assert!(!shared.active.load(Ordering::Acquire));
  }

  #[test]
  fn cancellation_rejects_late_body_without_growing_queue() {
    let shared = shared();
    shared.cancelled.store(true, Ordering::Release);
    assert!(shared.receive(b"late secret").is_err());
    assert!(shared.queue.lock().events.is_empty());
    assert_eq!(shared.queue.lock().bytes, 0);
  }
}
