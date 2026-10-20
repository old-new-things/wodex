//! HTTP policy and callback handling. Native code only translates libcurl calls.
use alloc::{ffi::CString, string::String, vec::Vec};
use core::{
  ffi::{c_char, c_void},
  sync::atomic::{AtomicBool, Ordering},
};

#[repr(C)]
#[derive(Clone, Copy)]
pub struct UrlPart {
  pub data: *const u8,
  pub len: usize,
  pub present: bool,
}
pub type Receive = unsafe extern "C" fn(*mut c_void, *const u8, usize) -> usize;
pub type Progress = unsafe extern "C" fn(*mut c_void) -> bool;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct HttpApi {
  pub init: unsafe extern "C" fn() -> bool,
  pub create: unsafe extern "C" fn() -> *mut c_void,
  pub destroy: unsafe extern "C" fn(*mut c_void),
  pub set_string: unsafe extern "C" fn(*mut c_void, u32, *const c_char) -> bool,
  pub set_number: unsafe extern "C" fn(*mut c_void, u32, i64) -> bool,
  pub set_body: unsafe extern "C" fn(*mut c_void, *const u8, usize) -> bool,
  pub append_header: unsafe extern "C" fn(*mut c_void, *const c_char) -> bool,
  pub callbacks: unsafe extern "C" fn(*mut c_void, *mut c_void, Receive, Receive, Progress) -> bool,
  pub perform: unsafe extern "C" fn(*mut c_void) -> u32,
  pub status: unsafe extern "C" fn(*mut c_void) -> u16,
  pub url_parse: unsafe extern "C" fn(*const c_char, *const c_char) -> *mut c_void,
  pub url_part: unsafe extern "C" fn(*mut c_void, u32) -> UrlPart,
  pub url_free: unsafe extern "C" fn(*mut c_void),
}

#[derive(Clone, Copy)]
pub struct Http {
  api: HttpApi,
}

#[derive(Debug, Default, Clone)]
pub struct UrlParts {
  pub scheme: String,
  pub host: String,
  pub port: String,
  pub path: String,
  pub has_user: bool,
  pub has_password: bool,
  pub has_fragment: bool,
  pub has_query: bool,
}

pub struct Request {
  pub url: String,
  pub post: bool,
  pub headers: Vec<String>,
  pub body: Vec<u8>,
  pub ca_bundle: String,
  pub user_agent: String,
  pub protocols: String,
  pub no_proxy: bool,
  pub timeout_seconds: u32,
  pub connect_timeout_seconds: u32,
  pub low_speed_bytes: u32,
  pub low_speed_seconds: u32,
  /// None permits all successful 2xx statuses; updates require exactly 200.
  pub body_status: Option<u16>,
  pub abort_rejected_body: bool,
}

impl Default for Request {
  fn default() -> Self {
    Self {
      url: String::new(),
      post: false,
      headers: Vec::new(),
      body: Vec::new(),
      ca_bundle: String::new(),
      user_agent: "Wodex/0.2".into(),
      protocols: "https".into(),
      no_proxy: false,
      timeout_seconds: 600,
      connect_timeout_seconds: 15,
      low_speed_bytes: 1,
      low_speed_seconds: 90,
      body_status: None,
      abort_rejected_body: false,
    }
  }
}

impl Drop for Request {
  fn drop(&mut self) {
    wipe(&mut self.body);
    for header in &mut self.headers {
      unsafe {
        wipe(header.as_mut_vec());
      }
    }
  }
}

pub fn wipe(bytes: &mut [u8]) {
  for byte in bytes {
    unsafe { core::ptr::write_volatile(byte, 0) };
  }
  core::sync::atomic::compiler_fence(Ordering::SeqCst);
}

#[derive(Debug, PartialEq, Eq)]
pub enum HttpError {
  Cancelled,
  Failure(String),
}

#[derive(Debug, Default)]
pub struct Response {
  pub http_status: u16,
  pub curl_code: u32,
}

struct Easy {
  api: HttpApi,
  handle: *mut c_void,
}
impl Drop for Easy {
  fn drop(&mut self) {
    unsafe { (self.api.destroy)(self.handle) };
  }
}

impl Http {
  /// # Safety
  /// Callbacks must remain valid and nonthrowing for this client and its copies.
  /// Separate handles must support concurrent use. init must be thread-safe.
  /// URL parts are borrowed until url_free; string options must copy their input.
  pub unsafe fn new(api: &HttpApi) -> Result<Self, String> {
    if !(api.init)() {
      return Err("Cannot initialize HTTPS networking.".into());
    }
    Ok(Self { api: *api })
  }

  pub fn parse_url(&self, value: &str) -> Result<UrlParts, String> {
    let base = c"";
    let value = CString::new(value).map_err(|_| String::from("Invalid URL."))?;
    let handle = unsafe { (self.api.url_parse)(base.as_ptr(), value.as_ptr()) };
    if handle.is_null() {
      return Err("Invalid URL.".into());
    }
    let part = |id| unsafe { (self.api.url_part)(handle, id) };
    let text = |id| {
      let value = part(id);
      if value.len == 0 {
        String::new()
      } else {
        String::from_utf8_lossy(unsafe { core::slice::from_raw_parts(value.data, value.len) })
          .into_owned()
      }
    };
    let parts = UrlParts {
      scheme: text(1),
      host: text(2),
      port: text(3),
      path: text(4),
      has_user: part(6).present,
      has_password: part(7).present,
      has_fragment: part(8).present,
      has_query: part(5).present,
    };
    unsafe { (self.api.url_free)(handle) };
    Ok(parts)
  }

  pub fn perform(
    &self,
    request: &Request,
    cancelled: &AtomicBool,
    sink: &mut dyn FnMut(&[u8]) -> Result<(), String>,
  ) -> Result<Response, HttpError> {
    if cancelled.load(Ordering::Acquire) {
      return Err(HttpError::Cancelled);
    }
    let handle = unsafe { (self.api.create)() };
    if handle.is_null() {
      return Err(HttpError::Failure("Cannot create the HTTPS request.".into()));
    }
    let easy = Easy { api: self.api, handle };
    let failure = || HttpError::Failure("Cannot configure the secure network request.".into());
    let string = |id, value: &str| -> Result<(), HttpError> {
      let value = CString::new(value).map_err(|_| failure())?;
      if unsafe { (self.api.set_string)(handle, id, value.as_ptr()) } {
        Ok(())
      } else {
        Err(failure())
      }
    };
    string(1, &request.url)?;
    if !request.ca_bundle.is_empty() {
      string(2, &request.ca_bundle)?;
    }
    string(3, &request.user_agent)?;
    if request.no_proxy {
      string(4, "*")?;
    }
    string(5, &request.protocols)?;
    string(6, "https")?;
    for (id, value) in [
      (1, i64::from(request.post)),
      (2, 0),
      (3, 1),
      (4, 2),
      (5, request.connect_timeout_seconds.into()),
      (6, request.timeout_seconds.into()),
      (7, request.low_speed_bytes.into()),
      (8, request.low_speed_seconds.into()),
      (9, 1),
      (10, 0),
      (11, 1),
    ] {
      if !unsafe { (self.api.set_number)(handle, id, value) } {
        return Err(failure());
      }
    }
    if request.post
      && !unsafe { (self.api.set_body)(handle, request.body.as_ptr(), request.body.len()) }
    {
      return Err(failure());
    }
    for header in &request.headers {
      if header.bytes().any(|byte| byte < 0x20 || byte == 0x7f) {
        return Err(failure());
      }
      let mut header = CString::new(header.as_str()).map_err(|_| failure())?.into_bytes_with_nul();
      let added = unsafe { (self.api.append_header)(handle, header.as_ptr().cast()) };
      wipe(&mut header);
      if !added {
        return Err(failure());
      }
    }
    let mut transfer = Transfer {
      http: *self,
      handle,
      cancelled,
      sink,
      error: None,
      header_bytes: 0,
      body_status: request.body_status,
      abort_rejected_body: request.abort_rejected_body,
    };
    if !unsafe {
      (self.api.callbacks)(
        handle,
        (&mut transfer as *mut Transfer<'_>).cast(),
        receive_body,
        receive_header,
        progress,
      )
    } {
      // A partly configured handle may already retain this callback context.
      // Destroy it while the complete transfer and its borrowed sink live.
      drop(easy);
      return Err(failure());
    }
    let code = unsafe { (self.api.perform)(handle) };
    let status = unsafe { (self.api.status)(handle) };
    drop(easy);
    if cancelled.load(Ordering::Acquire) {
      return Err(HttpError::Cancelled);
    }
    if let Some(error) = transfer.error {
      return Err(HttpError::Failure(error));
    }
    let response = Response { http_status: status, curl_code: code };
    Ok(response)
  }
}

struct Transfer<'a> {
  http: Http,
  handle: *mut c_void,
  cancelled: &'a AtomicBool,
  sink: &'a mut dyn FnMut(&[u8]) -> Result<(), String>,
  error: Option<String>,
  header_bytes: usize,
  body_status: Option<u16>,
  abort_rejected_body: bool,
}
unsafe extern "C" fn progress(context: *mut c_void) -> bool {
  let transfer = unsafe { &*(context.cast::<Transfer<'_>>()) };
  transfer.cancelled.load(Ordering::Acquire)
}
unsafe extern "C" fn receive_body(context: *mut c_void, data: *const u8, len: usize) -> usize {
  let transfer = unsafe { &mut *(context.cast::<Transfer<'_>>()) };
  if transfer.cancelled.load(Ordering::Acquire) {
    return 0;
  }
  let status = unsafe { (transfer.http.api.status)(transfer.handle) };
  if !(200..300).contains(&status)
    || transfer.body_status.is_some_and(|expected| status != expected)
  {
    return if transfer.abort_rejected_body { 0 } else { len };
  }
  let bytes = if len == 0 { &[] } else { unsafe { core::slice::from_raw_parts(data, len) } };
  match (transfer.sink)(bytes) {
    Ok(()) => len,
    Err(error) => {
      transfer.error = Some(error);
      0
    }
  }
}
unsafe extern "C" fn receive_header(context: *mut c_void, _data: *const u8, len: usize) -> usize {
  let transfer = unsafe { &mut *(context.cast::<Transfer<'_>>()) };
  if transfer.cancelled.load(Ordering::Acquire) {
    return 0;
  }
  if len > 65536 - transfer.header_bytes {
    transfer.error = Some("The response headers are too large.".into());
    return 0;
  }
  transfer.header_bytes += len;
  len
}

#[cfg(test)]
mod tests {
  use super::*;
  use alloc::{boxed::Box, collections::BTreeMap, vec};
  use core::ffi::CStr;

  #[derive(Default)]
  struct Fake {
    url: String,
    numbers: BTreeMap<u32, i64>,
    status: u16,
    context: *mut c_void,
    body: Option<Receive>,
    header: Option<Receive>,
    progress: Option<Progress>,
  }
  unsafe extern "C" fn init() -> bool {
    true
  }
  unsafe extern "C" fn create() -> *mut c_void {
    Box::into_raw(Box::new(Fake::default())).cast()
  }
  unsafe extern "C" fn destroy(handle: *mut c_void) {
    unsafe {
      drop(Box::<Fake>::from_raw(handle.cast()));
    }
  }
  unsafe extern "C" fn string(handle: *mut c_void, id: u32, text: *const c_char) -> bool {
    if id == 1 {
      unsafe { &mut *handle.cast::<Fake>() }.url =
        unsafe { CStr::from_ptr(text) }.to_str().unwrap().into();
    }
    true
  }
  unsafe extern "C" fn number(handle: *mut c_void, id: u32, value: i64) -> bool {
    unsafe { &mut *handle.cast::<Fake>() }.numbers.insert(id, value);
    true
  }
  unsafe extern "C" fn body(_: *mut c_void, _: *const u8, _: usize) -> bool {
    true
  }
  unsafe extern "C" fn append(_: *mut c_void, _: *const c_char) -> bool {
    true
  }
  unsafe extern "C" fn callbacks(
    handle: *mut c_void,
    context: *mut c_void,
    body: Receive,
    header: Receive,
    progress: Progress,
  ) -> bool {
    let fake = unsafe { &mut *handle.cast::<Fake>() };
    fake.context = context;
    fake.body = Some(body);
    fake.header = Some(header);
    fake.progress = Some(progress);
    true
  }
  unsafe extern "C" fn perform(handle: *mut c_void) -> u32 {
    let (context, body, header, progress, url, secured) = {
      let fake = unsafe { &*handle.cast::<Fake>() };
      (
        fake.context,
        fake.body.unwrap(),
        fake.header.unwrap(),
        fake.progress.unwrap(),
        fake.url.clone(),
        fake.numbers.get(&2) == Some(&0)
          && fake.numbers.get(&3) == Some(&1)
          && fake.numbers.get(&4) == Some(&2)
          && fake.numbers.get(&9) == Some(&1),
      )
    };
    if !secured {
      return 43;
    }
    let status = if url.ends_with("denied") {
      401
    } else if url.ends_with("redirect") {
      302
    } else {
      200
    };
    unsafe {
      (*handle.cast::<Fake>()).status = status;
    }
    if unsafe { progress(context) } {
      return 42;
    }
    let lines: Vec<Vec<u8>> = if url.ends_with("headers") {
      vec![vec![b'a'; 65537]]
    } else if status == 302 {
      vec![
        b"HTTP/1.1 100 Continue\r\n".to_vec(),
        b"Location: discarded\r\n".to_vec(),
        b"HTTP/1.1 302 Found\r\n".to_vec(),
        b"lOcAtIoN: \t/path?ok=1  \r\n".to_vec(),
      ]
    } else {
      vec![b"HTTP/1.1 200 OK\r\n".to_vec()]
    };
    for line in lines {
      if unsafe { header(context, line.as_ptr(), line.len()) } != line.len() {
        return 23;
      }
    }
    let bytes = b"secret response";
    if unsafe { body(context, bytes.as_ptr(), bytes.len()) } == bytes.len() {
      0
    } else {
      23
    }
  }
  unsafe extern "C" fn status(handle: *mut c_void) -> u16 {
    unsafe { (*handle.cast::<Fake>()).status }
  }
  unsafe extern "C" fn parse(_: *const c_char, _: *const c_char) -> *mut c_void {
    core::ptr::null_mut()
  }
  unsafe extern "C" fn part(_: *mut c_void, _: u32) -> UrlPart {
    UrlPart { data: core::ptr::null(), len: 0, present: false }
  }
  unsafe extern "C" fn free(_: *mut c_void) {}
  fn http() -> Http {
    unsafe {
      Http::new(&HttpApi {
        init,
        create,
        destroy,
        set_string: string,
        set_number: number,
        set_body: body,
        append_header: append,
        callbacks,
        perform,
        status,
        url_parse: parse,
        url_part: part,
        url_free: free,
      })
      .unwrap()
    }
  }
  fn request(path: &str) -> Request {
    let mut request = Request::default();
    request.url = path.into();
    request
  }

  #[test]
  fn non_success_bodies_never_reach_the_application_sink() {
    let cancelled = AtomicBool::new(false);
    let mut bytes = Vec::new();
    let response = http()
      .perform(&request("denied"), &cancelled, &mut |chunk| {
        bytes.extend_from_slice(chunk);
        Ok(())
      })
      .unwrap();
    assert_eq!(response.http_status, 401);
    assert!(bytes.is_empty());
    assert_eq!(response.curl_code, 0);
  }

  #[test]
  fn redirects_are_not_followed_and_bodies_are_not_exposed() {
    let cancelled = AtomicBool::new(false);
    let mut request = request("redirect");
    request.body_status = Some(200);
    request.abort_rejected_body = true;
    let response =
      http().perform(&request, &cancelled, &mut |_| panic!("redirect body exposed")).unwrap();
    assert_eq!(response.http_status, 302);
    assert_eq!(response.curl_code, 23);
  }

  #[test]
  fn body_sink_failure_stops_the_transfer_and_preserves_its_error() {
    let cancelled = AtomicBool::new(false);
    let error = http()
      .perform(&request("success"), &cancelled, &mut |_| Err("disk full".into()))
      .unwrap_err();
    assert_eq!(error, HttpError::Failure("disk full".into()));
  }

  #[test]
  fn oversized_headers_and_pre_cancelled_requests_fail_without_a_body() {
    let cancelled = AtomicBool::new(false);
    let error =
      http().perform(&request("headers"), &cancelled, &mut |_| panic!("body exposed")).unwrap_err();
    assert!(matches!(error, HttpError::Failure(error) if error.contains("headers")));
    cancelled.store(true, Ordering::Release);
    assert_eq!(
      http().perform(&request("success"), &cancelled, &mut |_| Ok(())).unwrap_err(),
      HttpError::Cancelled
    );
  }
}
