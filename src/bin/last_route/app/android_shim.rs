//! Android: the one libc function the cargo-quad-apk link leaves out.
//!
//! cargo-quad-apk links the game's shared library with a bare `ld` and the NDK sysroot, without the compiler's startup
//! objects. With NDK r23 and newer `atexit` is not exported by the system libc: clang's startup objects define it, so
//! the library loaded with "cannot locate symbol atexit". bionic does export `__cxa_atexit`, which is what `atexit` is
//! made of, so the library defines `atexit` itself. The handler never gets a library handle, so it runs at process exit
//! only, which is all `atexit` promises.

use std::os::raw::{c_int, c_void};

extern "C" {
    fn __cxa_atexit(f: unsafe extern "C" fn(*mut c_void), arg: *mut c_void, dso: *mut c_void) -> c_int;
}

/// # Safety
/// `f` is a C function pointer taking no arguments, as `atexit` documents.
#[no_mangle]
pub unsafe extern "C" fn atexit(f: unsafe extern "C" fn()) -> c_int {
    // a C call ignores an argument the callee does not read, so the no-argument handler is called through the
    // one-argument type `__cxa_atexit` wants
    let handler: unsafe extern "C" fn(*mut c_void) = std::mem::transmute(f);
    __cxa_atexit(handler, std::ptr::null_mut(), std::ptr::null_mut())
}
