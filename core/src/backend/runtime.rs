//! Thread and mutex ownership in Rust, backed by native platform primitives.
use alloc::{boxed::Box, string::String};
use core::{
  cell::UnsafeCell,
  ffi::c_void,
  ops::{Deref, DerefMut},
};

#[repr(C)]
#[derive(Clone, Copy)]
pub struct RuntimeApi {
  pub thread_spawn:
    unsafe extern "C" fn(unsafe extern "C" fn(*mut c_void), *mut c_void) -> *mut c_void,
  pub thread_join: unsafe extern "C" fn(*mut c_void),
  pub mutex_new: unsafe extern "C" fn() -> *mut c_void,
  pub mutex_lock: unsafe extern "C" fn(*mut c_void),
  pub mutex_unlock: unsafe extern "C" fn(*mut c_void),
  pub mutex_free: unsafe extern "C" fn(*mut c_void),
}

#[derive(Clone, Copy)]
pub struct Runtime {
  api: RuntimeApi,
}

impl Runtime {
  /// # Safety
  /// The callbacks must be valid, nonthrowing and callable from every worker.
  /// Spawn either starts exactly one invocation or returns null without invoking it;
  /// join waits for termination and frees the handle. Mutexes provide acquire/release.
  pub unsafe fn new(api: &RuntimeApi) -> Self {
    Self { api: *api }
  }

  pub fn mutex<T>(&self, value: T) -> Result<Mutex<T>, String> {
    let handle = unsafe { (self.api.mutex_new)() };
    if handle.is_null() {
      return Err("Cannot create a worker mutex.".into());
    }
    Ok(Mutex { runtime: *self, handle, value: UnsafeCell::new(value) })
  }

  pub fn spawn<F: FnOnce() + Send + 'static>(&self, callback: F) -> Result<JoinHandle, String> {
    type Work = Box<dyn FnOnce() + Send>;
    unsafe extern "C" fn run(context: *mut c_void) {
      let work: Box<Work> = unsafe { Box::from_raw(context.cast()) };
      work();
    }
    let context = Box::into_raw(Box::new(Box::new(callback) as Work)).cast();
    let handle = unsafe { (self.api.thread_spawn)(run, context) };
    if handle.is_null() {
      unsafe {
        drop(Box::<Work>::from_raw(context.cast()));
      }
      return Err("Cannot start the network worker.".into());
    }
    Ok(JoinHandle { runtime: *self, handle })
  }
}

pub struct JoinHandle {
  runtime: Runtime,
  handle: *mut c_void,
}
// A native handle may be transferred, but cannot be joined concurrently.
unsafe impl Send for JoinHandle {}
impl JoinHandle {
  pub fn join(self) {
    drop(self);
  }
}
impl Drop for JoinHandle {
  fn drop(&mut self) {
    unsafe { (self.runtime.api.thread_join)(self.handle) };
  }
}

pub struct Mutex<T> {
  runtime: Runtime,
  handle: *mut c_void,
  value: UnsafeCell<T>,
}
unsafe impl<T: Send> Send for Mutex<T> {}
unsafe impl<T: Send> Sync for Mutex<T> {}
impl<T> Mutex<T> {
  pub fn lock(&self) -> Guard<'_, T> {
    unsafe { (self.runtime.api.mutex_lock)(self.handle) };
    Guard { mutex: self, not_send: core::marker::PhantomData }
  }
}
impl<T> Drop for Mutex<T> {
  fn drop(&mut self) {
    unsafe { (self.runtime.api.mutex_free)(self.handle) };
  }
}
pub struct Guard<'a, T> {
  mutex: &'a Mutex<T>,
  not_send: core::marker::PhantomData<*mut ()>,
}
impl<T> Deref for Guard<'_, T> {
  type Target = T;
  fn deref(&self) -> &T {
    unsafe { &*self.mutex.value.get() }
  }
}
impl<T> DerefMut for Guard<'_, T> {
  fn deref_mut(&mut self) -> &mut T {
    unsafe { &mut *self.mutex.value.get() }
  }
}
impl<T> Drop for Guard<'_, T> {
  fn drop(&mut self) {
    unsafe { (self.mutex.runtime.api.mutex_unlock)(self.mutex.handle) };
  }
}

#[cfg(test)]
pub(crate) mod tests {
  use super::*;
  use alloc::sync::Arc;
  use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

  unsafe extern "C" fn spawn(
    entry: unsafe extern "C" fn(*mut c_void),
    context: *mut c_void,
  ) -> *mut c_void {
    let address = context as usize;
    match std::thread::Builder::new().spawn(move || unsafe { entry(address as *mut c_void) }) {
      Ok(handle) => Box::into_raw(Box::new(handle)).cast(),
      Err(_) => core::ptr::null_mut(),
    }
  }
  unsafe extern "C" fn join(handle: *mut c_void) {
    unsafe { Box::<std::thread::JoinHandle<()>>::from_raw(handle.cast()) }.join().unwrap();
  }
  unsafe extern "C" fn mutex_new() -> *mut c_void {
    Box::into_raw(Box::new(AtomicBool::new(false))).cast()
  }
  unsafe extern "C" fn mutex_lock(handle: *mut c_void) {
    let locked = unsafe { &*handle.cast::<AtomicBool>() };
    while locked.compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed).is_err() {
      std::thread::yield_now();
    }
  }
  unsafe extern "C" fn mutex_unlock(handle: *mut c_void) {
    unsafe { &*handle.cast::<AtomicBool>() }.store(false, Ordering::Release);
  }
  unsafe extern "C" fn mutex_free(handle: *mut c_void) {
    unsafe {
      drop(Box::<AtomicBool>::from_raw(handle.cast()));
    }
  }

  pub(crate) fn runtime() -> Runtime {
    unsafe {
      Runtime::new(&RuntimeApi {
        thread_spawn: spawn,
        thread_join: join,
        mutex_new,
        mutex_lock,
        mutex_unlock,
        mutex_free,
      })
    }
  }

  #[test]
  fn workers_share_a_movable_owned_mutex_and_join_before_drop() {
    let runtime = runtime();
    let counter = Arc::new(runtime.mutex(0usize).unwrap());
    let handles: alloc::vec::Vec<_> = (0..4)
      .map(|_| {
        let counter = Arc::clone(&counter);
        runtime
          .spawn(move || {
            for _ in 0..1000 {
              *counter.lock() += 1;
            }
          })
          .unwrap()
      })
      .collect();
    drop(handles);
    assert_eq!(*counter.lock(), 4000);
  }

  #[test]
  fn failed_spawn_reclaims_the_captured_closure() {
    unsafe extern "C" fn fail(_: unsafe extern "C" fn(*mut c_void), _: *mut c_void) -> *mut c_void {
      core::ptr::null_mut()
    }
    struct Captured(Arc<AtomicUsize>);
    impl Drop for Captured {
      fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::Relaxed);
      }
    }
    let drops = Arc::new(AtomicUsize::new(0));
    let captured = Captured(Arc::clone(&drops));
    let mut runtime = runtime();
    runtime.api.thread_spawn = fail;
    assert!(runtime.spawn(move || drop(captured)).is_err());
    assert_eq!(drops.load(Ordering::Relaxed), 1);
  }
}
