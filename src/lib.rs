// Copyright 2013-2015 The Rust Project Developers. See the COPYRIGHT
// file at the top-level directory of this distribution and at
// http://rust-lang.org/COPYRIGHT.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

//! Dynamic library facilities.
//!
//! A simple wrapper over the platform's dynamic library facilities
//!
//! Linux and macOS only

use std::{
    env,
    ffi::{CString, OsString},
    mem,
    path::{Path, PathBuf},
};

pub struct DynamicLibrary {
    handle: *mut u8,
}

impl Drop for DynamicLibrary {
    fn drop(&mut self) {
        match dl::check_for_errors_in(|| unsafe { dl::close(self.handle) }) {
            Ok(()) => {}
            Err(str) => panic!("{}", str),
        }
    }
}

impl DynamicLibrary {
    // FIXME (#12938): Until DST lands, we cannot decompose &str into
    // & and str, so we cannot usefully take ToCStr arguments by
    // reference (without forcing an additional & around &str). So we
    // are instead temporarily adding an instance for &Path, so that
    // we can take ToCStr as owned. When DST lands, the &Path instance
    // should be removed, and arguments bound by ToCStr should be
    // passed by reference. (Here: in the `open` method.)

    /// Lazily open a dynamic library. When passed None it gives a
    /// handle to the calling process
    pub fn open(filename: Option<&Path>) -> Result<DynamicLibrary, String> {
        let maybe_library = dl::open(filename.map(|path| path.as_os_str()));

        // The dynamic library must not be constructed if there is
        // an error opening the library so the destructor does not
        // run.
        match maybe_library {
            Err(err) => Err(err),
            Ok(handle) => Ok(DynamicLibrary { handle }),
        }
    }

    /// Prepends a path to this process's search path for dynamic libraries
    pub fn prepend_search_path(path: &Path) {
        let mut search_path = DynamicLibrary::search_path();
        search_path.insert(0, path.to_path_buf());
        // TODO: Audit that the environment access only happens in single-threaded code.
        unsafe {
            env::set_var(
                DynamicLibrary::envvar(),
                DynamicLibrary::create_path(&search_path),
            )
        };
    }

    /// From a slice of paths, create a new vector which is suitable to be an
    /// environment variable for this platforms dylib search path.
    pub fn create_path(path: &[PathBuf]) -> OsString {
        let mut newvar = OsString::new();
        for (i, path) in path.iter().enumerate() {
            if i > 0 {
                newvar.push(DynamicLibrary::separator());
            }
            newvar.push(path);
        }
        newvar
    }

    /// Returns the environment variable for this process's dynamic library
    /// search path
    pub const fn envvar() -> &'static str {
        if cfg!(target_os = "macos") {
            "DYLD_LIBRARY_PATH"
        } else {
            "LD_LIBRARY_PATH"
        }
    }

    const fn separator() -> &'static str {
        ":"
    }

    /// Returns the current search path for dynamic libraries being used by this
    /// process
    pub fn search_path() -> Vec<PathBuf> {
        match env::var_os(DynamicLibrary::envvar()) {
            Some(var) => env::split_paths(&var).collect(),
            None => Vec::new(),
        }
    }

    /// Access the value at the symbol of the dynamic library
    #[allow(clippy::missing_safety_doc)]
    pub unsafe fn symbol<T>(&self, symbol: &str) -> Result<*mut T, String> {
        unsafe {
            // This function should have a lifetime constraint of 'a on
            // T but that feature is still unimplemented

            let Ok(raw_string) = CString::new(symbol) else {
                return Err(format!("failed to access `{symbol}`"));
            };
            let maybe_symbol_value =
                dl::check_for_errors_in(|| dl::symbol(self.handle, raw_string.as_ptr()));

            // The value must not be constructed if there is an error so
            // the destructor does not run.
            match maybe_symbol_value {
                Err(err) => Err(err),
                Ok(symbol_value) => Ok(mem::transmute::<*mut u8, *mut T>(symbol_value)),
            }
        }
    }
}

#[cfg(any(
    target_os = "linux",
    target_os = "android",
    target_os = "macos",
    target_os = "ios",
    target_os = "freebsd",
    target_os = "dragonfly",
    target_os = "openbsd"
))]
mod dl {
    use std::{
        ffi::{CStr, CString, OsStr},
        os::unix::ffi::OsStrExt,
        ptr, str,
    };

    pub fn open(filename: Option<&OsStr>) -> Result<*mut u8, String> {
        check_for_errors_in(|| unsafe {
            match filename {
                Some(filename) => open_external(filename),
                None => open_internal(),
            }
        })
    }

    const LAZY: libc::c_int = 1;

    unsafe fn open_external(filename: &OsStr) -> *mut u8 {
        unsafe {
            let Ok(s) = CString::new(filename.as_bytes()) else {
                panic!("failed to open external `{}`", filename.to_string_lossy());
            };
            dlopen(s.as_ptr(), LAZY) as *mut u8
        }
    }

    unsafe fn open_internal() -> *mut u8 {
        unsafe { dlopen(ptr::null(), LAZY) as *mut u8 }
    }

    pub fn check_for_errors_in<T, F>(f: F) -> Result<T, String>
    where
        F: FnOnce() -> T,
    {
        unsafe {
            let result = f();

            let last_error = dlerror() as *const libc::c_char;
            if last_error.is_null() {
                Ok(result)
            } else {
                let s = CStr::from_ptr(last_error).to_bytes();
                let error = str::from_utf8(s)
                    .map_err(|e| format!("failed to check for errors: {e}"))?
                    .to_string();
                Err(error)
            }
        }
    }

    pub unsafe fn symbol(handle: *mut u8, symbol: *const libc::c_char) -> *mut u8 {
        unsafe { dlsym(handle as *mut libc::c_void, symbol) as *mut u8 }
    }
    pub unsafe fn close(handle: *mut u8) {
        unsafe {
            dlclose(handle as *mut libc::c_void);
        }
    }

    unsafe extern "C" {
        fn dlopen(filename: *const libc::c_char, flag: libc::c_int) -> *mut libc::c_void;
        fn dlerror() -> *mut libc::c_char;
        fn dlsym(handle: *mut libc::c_void, symbol: *const libc::c_char) -> *mut libc::c_void;
        fn dlclose(handle: *mut libc::c_void) -> libc::c_int;
    }
}

#[cfg(all(test, not(target_os = "ios")))]
mod test {
    use std::{mem, path::Path};

    use super::*;

    #[test]
    #[cfg_attr(target_os = "linux", ignore)]
    fn test_loading_cosine() {
        // The math library does not need to be loaded since it is already
        // statically linked in
        let libm = match DynamicLibrary::open(None) {
            Err(error) => panic!("Could not load self as module: {}", error),
            Ok(libm) => libm,
        };

        let cosine: extern "C" fn(libc::c_double) -> libc::c_double = unsafe {
            match libm.symbol("cos") {
                Err(error) => panic!("Could not load function cos: {}", error),
                Ok(cosine) => mem::transmute::<*mut u8, extern "C" fn(f64) -> f64>(cosine),
            }
        };

        let argument = 0.0;
        let expected_result = 1.0;
        let result = cosine(argument);
        assert_eq!(
            result, expected_result,
            "cos({}) != {} but equaled {} instead",
            argument, expected_result, result
        )
    }

    #[test]
    #[cfg(any(
        target_os = "linux",
        target_os = "macos",
        target_os = "freebsd",
        target_os = "dragonfly",
        target_os = "openbsd"
    ))]
    fn test_errors_do_not_crash() {
        // Open /dev/null as a library to get an error, and make sure
        // that only causes an error, and not a crash.
        let path = Path::new("/dev/null");
        match DynamicLibrary::open(Some(path)) {
            Err(_) => {}
            Ok(_) => panic!("Successfully opened the empty library."),
        }
    }
}
