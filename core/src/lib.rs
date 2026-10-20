#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

mod ffi;
mod state;
mod stream;
mod update;

pub use state::WodexCore;
pub use update::WodexSha256;

// libnx provides these C runtime functions on Switch. Keeping allocation here
// lets the protocol/state crate work without a custom Rust standard library.
#[cfg(not(feature = "std"))]
mod runtime {
  use core::alloc::{GlobalAlloc, Layout};
  use core::ffi::c_void;

  struct LibcAllocator;

  extern "C" {
    #[cfg(not(target_os = "horizon"))]
    fn posix_memalign(ptr: *mut *mut c_void, alignment: usize, size: usize) -> i32;
    #[cfg(target_os = "horizon")]
    fn memalign(alignment: usize, size: usize) -> *mut c_void;
    fn free(ptr: *mut c_void);
    fn abort() -> !;
  }

  unsafe impl GlobalAlloc for LibcAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
      let alignment = layout.align().max(core::mem::size_of::<usize>());
      #[cfg(target_os = "horizon")]
      {
        memalign(alignment, layout.size().max(1)).cast()
      }
      #[cfg(not(target_os = "horizon"))]
      {
        let mut ptr = core::ptr::null_mut();
        if posix_memalign(&mut ptr, alignment, layout.size().max(1)) == 0 {
          ptr.cast()
        } else {
          core::ptr::null_mut()
        }
      }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, _layout: Layout) {
      free(ptr.cast());
    }
  }

  #[global_allocator]
  static ALLOCATOR: LibcAllocator = LibcAllocator;

  #[cfg(not(test))]
  #[panic_handler]
  fn panic(_info: &core::panic::PanicInfo<'_>) -> ! {
    unsafe { abort() }
  }
}
