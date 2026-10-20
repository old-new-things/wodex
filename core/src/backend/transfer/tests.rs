use super::*;
use alloc::{vec, vec::Vec};
use std::cell::RefCell;

const AUTHORITY: &str = "192.168.1.42:54321";
const CODE: &str = "0123456789abcdef0123456789abcdef";

fn parse(text: &str) -> Result<Request<'_>, u16> {
  Request::parse(text.as_bytes(), AUTHORITY, CODE)
}

fn get(path: &str, headers: &str) -> String {
  format!("GET {path} HTTP/1.1\r\nHost: {AUTHORITY}\r\n{headers}\r\n")
}

fn post(channel: &str, body: &str) -> String {
  format!(
    "POST /api/{channel} HTTP/1.1\r\nHost: {AUTHORITY}\r\nAuthorization: Bearer {CODE}\r\n\
     Content-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
    body.len()
  )
}

#[test]
fn public_assets_need_no_code_but_every_api_route_does() {
  assert!(parse(&get("/", "")).is_ok());
  assert!(parse(&get("/transfer.js", "")).is_ok());
  assert_eq!(parse(&get("/api/config", "")), Err(401));
  assert_eq!(parse(&get("/api/sessions", "")), Err(401));
  assert_eq!(parse(&get("/api/status", "")), Err(401));
  assert_eq!(parse(&get("/api/unknown", "")), Err(401));
  assert!(parse(&get("/api/config", &format!("Authorization: Bearer {CODE}\r\n"))).is_ok());
  for value in [format!("Bearer {CODE}0"), format!("Bearer {}", &CODE[..31]), "Basic abc".into()] {
    assert_eq!(parse(&get("/api/config", &format!("Authorization: {value}\r\n"))), Err(401));
  }
  let mut wrong = CODE.to_owned();
  wrong.replace_range(31.., "0");
  assert_eq!(parse(&get("/api/config", &format!("Authorization: Bearer {wrong}\r\n"))), Err(401));
}

#[test]
fn reject_rebinding_foreign_origins_and_cross_site_browser_requests() {
  for host in ["evil.example", "192.168.1.42", "192.168.1.42:54322", "localhost:54321"] {
    let text = get("/", "").replace(AUTHORITY, host);
    assert_eq!(parse(&text), Err(403));
  }
  for origin in ["null", "https://evil.example", "http://192.168.1.42:54322"] {
    assert_eq!(parse(&get("/", &format!("Origin: {origin}\r\n"))), Err(403));
  }
  for site in ["cross-site", "same-site", "unexpected"] {
    assert_eq!(parse(&get("/", &format!("Sec-Fetch-Site: {site}\r\n"))), Err(403));
  }
  assert!(parse(&get(
    "/",
    &format!("Origin: http://{AUTHORITY}\r\nSec-Fetch-Site: same-origin\r\n")
  ))
  .is_ok());
  assert!(parse(&get("/", "Sec-Fetch-Site: none\r\n")).is_ok());
}

#[test]
fn reject_request_smuggling_and_header_injection() {
  let requests = [
    get("/", "Host: 192.168.1.42:54321\r\n"),
    get("/", "Content-Length: 0\r\ncontent-length: 0\r\n"),
    get("/", "Transfer-Encoding: chunked\r\n"),
    get("/", "Transfer-Encoding: identity\r\n"),
    get("/", "Expect: 100-continue\r\n"),
    get("/", "Bad Header: value\r\n"),
    get("/", " X-Header: value\r\n"),
    get("/", "X-Header: value\ninjected\r\n"),
    get("/", "X-Header: value\0hidden\r\n"),
    get("/", "").replace("HTTP/1.1", "HTTP/1.0"),
    get("/", "").replace("GET / ", "GET  / "),
  ];
  for request in requests {
    assert_eq!(parse(&request), Err(400), "{request:?}");
  }
  assert_eq!(parse(&get("/", "").replace("\r\n", "\n")), Err(400));
}

#[test]
fn enforce_bounded_exact_document_lengths_and_json_media_type() {
  let good = post("config", "{}");
  let header = good.split_once("\r\n\r\n").unwrap().0.to_owned() + "\r\n\r\n";
  assert_eq!(parse(&header).unwrap().content_length, 2);
  assert_eq!(parse(&header.replace("Content-Length: 2\r\n", "")), Err(411));
  for value in ["+2", "-2", "2, 2", "2 2", ""] {
    assert_eq!(
      parse(&header.replace("Content-Length: 2", &format!("Content-Length: {value}"))),
      Err(400)
    );
  }
  for value in ["4194305", "999999999999999999999999999999999"] {
    assert_eq!(
      parse(&header.replace("Content-Length: 2", &format!("Content-Length: {value}"))),
      Err(413)
    );
  }
  assert!(parse(&header.replace("Content-Length: 2", "Content-Length: 4194304")).is_ok());
  assert_eq!(parse(&header.replace("application/json", "text/plain")), Err(415));
  assert!(parse(&header.replace("application/json", "application/json; charset=utf-8")).is_ok());
  assert_eq!(
    parse(&header.replace("Content-Type:", "Content-Encoding: gzip\r\nContent-Type:")),
    Err(415)
  );
  assert_eq!(parse(&get("/", "Content-Length: 1\r\n")), Err(400));
  assert_eq!(parse(&header.replace("Content-Length: 2", "Content-Length: 0")), Err(400));
}

#[test]
fn bound_header_count_and_require_plain_origin_form_requests() {
  let headers = (0..64).map(|index| format!("X-{index}: x\r\n")).collect::<String>();
  assert_eq!(parse(&get("/", &headers)), Err(400));
  assert_eq!(parse(&get("http://192.168.1.42:54321/", "")), Err(400));
  assert_eq!(parse(&get("/", "").replace("GET ", "OPTIONS ")), Err(405));
  let oversized = get("/", &format!("X-Long: {}\r\n", "x".repeat(8192)));
  assert_eq!(parse(&oversized), Err(400));
}

#[test]
fn responses_disable_caching_framing_and_cross_origin_access() {
  let response = Response::download(br#"{"api_key":"example-key"}"#, true);
  let header = response.header();
  for expected in [
    "Cache-Control: no-store\r\n",
    "Connection: close\r\n",
    "X-Content-Type-Options: nosniff\r\n",
    "X-Frame-Options: DENY\r\n",
    "Cross-Origin-Resource-Policy: same-origin\r\n",
    "frame-ancestors 'none'",
    "Content-Disposition: attachment; filename=\"wodex-config.json\"\r\n",
    &format!("Content-Length: {}\r\n", response.body.len()),
  ] {
    assert!(header.contains(expected), "{expected}");
  }
  assert!(!header.contains("Access-Control-Allow"));
  assert!(!header.contains("example-key"));
}

#[derive(Default)]
struct Wire {
  request: Vec<u8>,
  position: usize,
  response: Vec<u8>,
  now: u64,
  closed: usize,
  reads: usize,
  half_closed: usize,
}

std::thread_local! {
  static WIRE: RefCell<Wire> = RefCell::new(Wire::default());
}

unsafe extern "C" fn local_ipv4(output: *mut u8) -> bool {
  unsafe { core::ptr::copy_nonoverlapping([127, 0, 0, 1].as_ptr(), output, 4) };
  true
}
unsafe extern "C" fn random(output: *mut u8, length: usize) -> bool {
  unsafe { core::ptr::write_bytes(output, 0x42, length) };
  true
}
unsafe extern "C" fn listen(_: *const u8, port: *mut u16) -> i32 {
  unsafe { *port = 12345 };
  8
}
unsafe extern "C" fn accept(_: i32) -> i32 {
  -2
}
unsafe extern "C" fn receive(_: i32, output: *mut u8, length: usize) -> i32 {
  WIRE.with_borrow_mut(|wire| {
    let count = length.min(37).min(wire.request.len() - wire.position);
    unsafe {
      core::ptr::copy_nonoverlapping(wire.request.as_ptr().add(wire.position), output, count)
    };
    wire.position += count;
    wire.reads += 1;
    count as i32
  })
}
unsafe extern "C" fn send(_: i32, bytes: *const u8, length: usize) -> i32 {
  WIRE.with_borrow_mut(|wire| {
    let count = length.min(61);
    wire.response.extend_from_slice(unsafe { core::slice::from_raw_parts(bytes, count) });
    count as i32
  })
}
unsafe extern "C" fn wait(_: i32, writable: bool, timeout: u32) -> i32 {
  WIRE.with_borrow_mut(|wire| {
    wire.now += 1;
    if writable || wire.position < wire.request.len() {
      1
    } else {
      wire.now += timeout as u64;
      0
    }
  })
}
unsafe extern "C" fn close(_: i32) {
  WIRE.with_borrow_mut(|wire| wire.closed += 1);
}
unsafe extern "C" fn now_ms() -> u64 {
  WIRE.with_borrow(|wire| wire.now)
}
unsafe extern "C" fn shutdown_write(_: i32) -> bool {
  WIRE.with_borrow_mut(|wire| wire.half_closed += 1);
  true
}
fn api() -> TransferApi {
  TransferApi {
    local_ipv4,
    random,
    listen,
    accept,
    receive,
    send,
    wait,
    close,
    now_ms,
    shutdown_write,
  }
}
fn shared() -> Mutex<Shared> {
  super::super::runtime::tests::runtime()
    .mutex(Shared {
      status: TransferStatus { supported: true, active: true, ..TransferStatus::default() },
      exports: Some(TransferExports {
        config: "{\"api_key\":\"saved-example-key\"}".into(),
        sessions: "{\"conversations\":[]}".into(),
      }),
      incoming: None,
    })
    .unwrap()
}
fn request_bytes(request: Vec<u8>, shared: &Mutex<Shared>) -> String {
  WIRE.with_borrow_mut(|wire| *wire = Wire { request, ..Wire::default() });
  let socket = Socket { api: api(), fd: 9 };
  handle_request(&socket, AUTHORITY, CODE, shared, &AtomicBool::new(false), 900_000);
  WIRE.with_borrow(|wire| String::from_utf8(wire.response.clone()).unwrap())
}
fn request(text: String, shared: &Mutex<Shared>) -> String {
  request_bytes(text.into_bytes(), shared)
}

#[test]
fn downloads_include_saved_api_key_only_after_pairing() {
  let shared = shared();
  let response = request(get("/api/config", ""), &shared);
  assert!(response.starts_with("HTTP/1.1 401"));
  assert!(!response.contains("saved-example-key"));
  let response = request(get("/api/config", &format!("Authorization: Bearer {CODE}\r\n")), &shared);
  assert!(response.starts_with("HTTP/1.1 200"));
  assert!(response.ends_with("{\"api_key\":\"saved-example-key\"}"));
  assert!(WIRE.with_borrow(|wire| wire.reads) > 1);
}

#[test]
fn uploads_queue_once_and_wait_for_device_confirmation() {
  let shared = shared();
  let response = request(post("sessions", "{\"conversations\":[]}"), &shared);
  assert!(response.starts_with("HTTP/1.1 202"));
  assert!(response.contains("\"pending\":true"));
  {
    let mut guard = shared.lock();
    assert!(guard.status.pending);
    let import = guard.incoming.take().unwrap();
    assert_eq!(import.channel, TransferChannel::Sessions);
    assert_eq!(import.data, "{\"conversations\":[]}");
    assert_eq!(guard.exports.as_ref().unwrap().sessions, "{\"conversations\":[]}");
  }
  let response = request(post("config", "{\"settings\":{}}"), &shared);
  assert!(response.starts_with("HTTP/1.1 409"));
  assert!(shared.lock().incoming.is_none());
  let response = request(get("/api/status", &format!("Authorization: Bearer {CODE}\r\n")), &shared);
  assert!(response.contains("\"pending\":true"));
  assert!(!response.contains(CODE));
}

#[test]
fn rejected_posts_half_close_and_drain_in_flight_body_with_a_deadline() {
  let shared = shared();
  let body = "x".repeat(1024);
  let text = post("config", &body).replace("application/json", "text/plain");
  let response = request(text, &shared);
  assert!(response.starts_with("HTTP/1.1 415"));
  WIRE.with_borrow(|wire| {
    assert_eq!(wire.half_closed, 1);
    assert_eq!(wire.position, wire.request.len());
    assert!(wire.now < DRAIN_TIMEOUT_MS + 200);
  });
  assert!(shared.lock().incoming.is_none());

  shared.lock().status.pending = true;
  let response = request(post("config", &body), &shared);
  assert!(response.starts_with("HTTP/1.1 409"));
  WIRE.with_borrow(|wire| {
    assert_eq!(wire.half_closed, 1);
    assert_eq!(wire.position, wire.request.len());
    assert!(wire.now < DRAIN_TIMEOUT_MS + 200);
  });
  assert!(shared.lock().incoming.is_none());
}

#[test]
fn malformed_or_truncated_bodies_never_create_an_import() {
  let shared = shared();
  let mut invalid_utf8 = post("config", "x").into_bytes();
  *invalid_utf8.last_mut().unwrap() = 0xff;
  assert!(request_bytes(invalid_utf8, &shared).starts_with("HTTP/1.1 400"));
  let request_text = post("config", "{}").replace("Content-Length: 2", "Content-Length: 200");
  assert!(request(request_text, &shared).is_empty());
  assert!(WIRE.with_borrow(|wire| wire.now) >= BODY_TIMEOUT_MS);
  assert!(shared.lock().incoming.is_none());
  assert!(!shared.lock().status.pending);
}

#[test]
fn malicious_slow_headers_and_stop_are_bounded() {
  let shared = shared();
  assert!(request("GET / HTTP/1.1\r\n".into(), &shared).is_empty());
  assert!(WIRE.with_borrow(|wire| wire.now) >= HEADER_TIMEOUT_MS);
  WIRE.with_borrow_mut(|wire| *wire = Wire::default());
  let socket = Socket { api: api(), fd: 10 };
  handle_request(&socket, AUTHORITY, CODE, &shared, &AtomicBool::new(true), 900_000);
  assert_eq!(WIRE.with_borrow(|wire| wire.now), 0);
}

#[test]
fn oversized_headers_and_body_suffix_are_rejected() {
  let shared = shared();
  let oversized = format!("GET / HTTP/1.1\r\nX-Long: {}", "x".repeat(HEADER_LIMIT));
  assert!(request(oversized, &shared).starts_with("HTTP/1.1 431"));
  let extra = post("config", "{}").replace("Content-Length: 2", "Content-Length: 1");
  assert!(request(extra, &shared).starts_with("HTTP/1.1 400"));
  assert!(shared.lock().incoming.is_none());
}

#[test]
fn only_fixed_asset_paths_are_served() {
  let shared = shared();
  let response = request(get("/", ""), &shared);
  assert!(response.starts_with("HTTP/1.1 200"));
  assert!(response.contains("text/html"));
  for path in [
    "/../state.json",
    "/state.json",
    "/%2e%2e/state.json",
    "/?code=secret",
    "/romfs/transfer/index.html",
  ] {
    assert!(request(get(path, ""), &shared).starts_with("HTTP/1.1 404"), "{path}");
  }
}

#[test]
fn unsupported_and_oversized_start_fail_without_listening() {
  let runtime = super::super::runtime::tests::runtime();
  let mut transfer = unsafe { Transfer::new(runtime, core::ptr::null()) }.unwrap();
  assert!(!transfer.status().supported);
  let exports = || TransferExports { config: "{}".into(), sessions: "{}".into() };
  assert!(transfer.start(exports()).is_err());
  assert!(!transfer.active());
  let api = api();
  let mut transfer = unsafe { Transfer::new(runtime, &api) }.unwrap();
  let oversized = TransferExports {
    config: String::from_utf8(vec![b'x'; MAX_SAVED_BYTES + 1]).unwrap(),
    sessions: "{}".into(),
  };
  assert!(transfer.start(oversized).is_err());
  assert!(!transfer.active());
}

#[test]
fn import_resolution_refreshes_snapshots_and_stop_erases_transient_state() {
  let runtime = super::super::runtime::tests::runtime();
  let api = api();
  let mut transfer = unsafe { Transfer::new(runtime, &api) }.unwrap();
  {
    let mut shared = transfer.shared.lock();
    shared.status.active = true;
    shared.status.pending = true;
    shared.status.code = CODE.into();
    shared.status.url = format!("http://{AUTHORITY}");
    shared.exports = Some(TransferExports { config: "old-key".into(), sessions: "old".into() });
    shared.incoming =
      Some(TransferImport { channel: TransferChannel::Config, data: "new-key".into() });
  }
  transfer.resolve_import(
    "Configuration imported.".into(),
    Some(TransferExports { config: "new-key".into(), sessions: "old".into() }),
  );
  assert!(!transfer.status().pending);
  assert_eq!(transfer.shared.lock().exports.as_ref().unwrap().config, "new-key");
  assert!(transfer.take_import().is_none());
  transfer.stop();
  let shared = transfer.shared.lock();
  assert!(!shared.status.active);
  assert!(shared.status.code.is_empty());
  assert!(shared.status.url.is_empty());
  assert!(shared.exports.is_none());
  assert!(shared.incoming.is_none());
}

#[test]
fn session_expiry_closes_listener_and_discards_unconfirmed_data() {
  WIRE.with_borrow_mut(|wire| *wire = Wire::default());
  let shared = Arc::new(shared());
  {
    let mut state = shared.lock();
    state.status.code = CODE.into();
    state.status.pending = true;
    state.incoming =
      Some(TransferImport { channel: TransferChannel::Config, data: "secret".into() });
  }
  serve(
    Socket { api: api(), fd: 8 },
    AUTHORITY.into(),
    CODE.into(),
    Arc::clone(&shared),
    Arc::new(AtomicBool::new(false)),
  );
  let state = shared.lock();
  assert!(!state.status.active);
  assert!(!state.status.pending);
  assert!(state.status.code.is_empty());
  assert!(state.status.message.contains("expired"));
  assert!(state.exports.is_none());
  assert!(state.incoming.is_none());
  assert_eq!(WIRE.with_borrow(|wire| wire.closed), 1);
  assert!(WIRE.with_borrow(|wire| wire.now) >= SESSION_TIMEOUT_MS);
}
