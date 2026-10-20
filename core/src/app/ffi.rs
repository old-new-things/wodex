//! Narrow ownership boundary used by the native process entry point.
use super::{canvas::RenderApi, host::HostApi, host::NativeHost, App, Canvas};
use alloc::{boxed::Box, ffi::CString, string::String};
use core::ffi::{c_char, CStr};

pub struct WodexApp {
  app: App<NativeHost>,
  canvas: Canvas,
}

unsafe fn report_error(output: *mut *mut c_char, message: &str) {
  if let Some(output) = output.as_mut() {
    *output = CString::new(message.replace('\0', " "))
      .map(CString::into_raw)
      .unwrap_or(core::ptr::null_mut());
  }
}

#[no_mangle]
pub unsafe extern "C" fn wodex_app_new(
  host: *const HostApi,
  render: *const RenderApi,
  version: *const c_char,
  demo: bool,
  show_updates: bool,
  error: *mut *mut c_char,
) -> *mut WodexApp {
  if let Some(error) = error.as_mut() {
    *error = core::ptr::null_mut();
  }
  let result: Result<WodexApp, String> = (|| {
    let (Some(host), Some(render)) = (host.as_ref(), render.as_ref()) else {
      return Err("Missing native application services.".into());
    };
    if version.is_null() {
      return Err("Missing application version.".into());
    }
    let version = CStr::from_ptr(version).to_str().map_err(|_| "Invalid application version.")?;
    let canvas = Canvas::new(render)?;
    let app = App::new(NativeHost::new(host), version, demo, show_updates)?;
    Ok(WodexApp { app, canvas })
  })();
  match result {
    Ok(app) => Box::into_raw(Box::new(app)),
    Err(message) => {
      report_error(error, &message);
      core::ptr::null_mut()
    }
  }
}

#[no_mangle]
pub unsafe extern "C" fn wodex_app_tick(app: *mut WodexApp, error: *mut *mut c_char) -> bool {
  if let Some(error) = error.as_mut() {
    *error = core::ptr::null_mut();
  }
  if let Some(app) = app.as_mut() {
    app.app.tick();
    true
  } else {
    report_error(error, "The application is not initialized.");
    false
  }
}

#[no_mangle]
pub unsafe extern "C" fn wodex_app_render(app: *mut WodexApp, error: *mut *mut c_char) -> bool {
  if let Some(error) = error.as_mut() {
    *error = core::ptr::null_mut();
  }
  let result = match app.as_mut() {
    Some(app) => app.app.render(&mut app.canvas),
    None => Err("The application is not initialized.".into()),
  };
  if let Err(message) = result {
    report_error(error, &message);
    false
  } else {
    true
  }
}

#[no_mangle]
pub unsafe extern "C" fn wodex_app_running(app: *const WodexApp) -> bool {
  app.as_ref().is_some_and(|app| app.app.running())
}

#[no_mangle]
pub unsafe extern "C" fn wodex_app_free(app: *mut WodexApp) {
  if !app.is_null() {
    drop(Box::from_raw(app));
  }
}
