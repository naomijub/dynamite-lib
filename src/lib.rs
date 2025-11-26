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
        if cfg!(windows) {
            "PATH"
        } else if cfg!(target_os = "macos") {
            "DYLD_LIBRARY_PATH"
        } else {
            "LD_LIBRARY_PATH"
        }
    }

    const fn separator() -> &'static str {
        if cfg!(windows) { ";" } else { ":" }
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

#[cfg(all(test, not(target_os = "ios")))]
mod test {
    #[allow(unused)]
    use std::{mem, path::Path};

    use super::*;

    #[test]
    #[cfg_attr(any(windows, target_os = "android"), ignore)] // FIXME #8818, #10379
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
        if result != expected_result {
            panic!(
                "cos({}) != {} but equaled {} instead",
                argument, expected_result, result
            )
        }
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

#[cfg(target_os = "windows")]
mod dl {
    use std::{
        ffi::{OsStr, OsString},
        iter::Iterator,
        ops::FnOnce,
        option::Option::{self, None, Some},
        os::windows::{ffi::OsStringExt, prelude::*},
        ptr,
        result::{
            Result,
            Result::{Err, Ok},
        },
        string::String,
        vec::Vec,
    };

    use windows_sys::Win32::{
        Foundation::{
            BOOL, ERROR_CALL_NOT_IMPLEMENTED, FORMAT_MESSAGE_FROM_SYSTEM,
            FORMAT_MESSAGE_IGNORE_INSERTS, GetLastError,
        },
        Globalization::FormatMessageW,
        System::Diagnostics::Debug::SetThreadErrorMode,
    };

    pub fn open(filename: Option<&OsStr>) -> Result<*mut u8, String> {
        // disable "dll load failed" error dialog.
        let mut use_thread_mode = true;
        let prev_error_mode = unsafe {
            // SEM_FAILCRITICALERRORS 0x01
            let new_error_mode = 1;
            let mut prev_error_mode = 0;
            // Windows >= 7 supports thread error mode.
            let result = SetThreadErrorMode(new_error_mode, &mut prev_error_mode);
            if result == 0 {
                let err = GetLastError();
                if err as libc::c_int == ERROR_CALL_NOT_IMPLEMENTED {
                    use_thread_mode = false;
                    // SetThreadErrorMode not found. use fallback solution:
                    // SetErrorMode() Note that SetErrorMode is process-wide so
                    // this can cause race condition!  However, since even
                    // Windows APIs do not care of such problem (#20650), we
                    // just assume SetErrorMode race is not a great deal.
                    prev_error_mode = SetErrorMode(new_error_mode);
                }
            }
            prev_error_mode
        };

        unsafe {
            SetLastError(0);
        }

        let result = match filename {
            Some(filename) => {
                let filename_str: Vec<_> =
                    filename.encode_wide().chain(Some(0).into_iter()).collect();
                let result = unsafe { LoadLibraryW(filename_str.as_ptr() as *const libc::c_void) };
                // beware: Vec/String may change errno during drop!
                // so we get error here.
                if result == ptr::null_mut() {
                    let errno = GetLastError();
                    Err(win_error_string(errno))
                } else {
                    Ok(result as *mut u8)
                }
            }
            None => {
                let mut handle = ptr::null_mut();
                let succeeded =
                    unsafe { GetModuleHandleExW(0 as libc::c_uint, ptr::null(), &mut handle) };
                if succeeded == 0 {
                    let errno = GetLastError();
                    Err(win_error_string(errno))
                } else {
                    Ok(handle as *mut u8)
                }
            }
        };

        unsafe {
            if use_thread_mode {
                SetThreadErrorMode(prev_error_mode, ptr::null_mut());
            } else {
                SetErrorMode(prev_error_mode);
            }
        }

        result
    }

    pub fn check_for_errors_in<T, F>(f: F) -> Result<T, String>
    where
        F: FnOnce() -> T,
    {
        unsafe {
            SetLastError(0);

            let result = f();

            let error = GetLastError();
            if 0 == error {
                Ok(result)
            } else {
                Err(format!("Error code {}", error))
            }
        }
    }

    pub unsafe fn symbol(handle: *mut u8, symbol: *const libc::c_char) -> *mut u8 {
        GetProcAddress(handle as *mut libc::c_void, symbol) as *mut u8
    }
    pub unsafe fn close(handle: *mut u8) {
        FreeLibrary(handle as *mut libc::c_void);
        ()
    }

    #[allow(non_snake_case)]
    unsafe extern "system" {
        fn SetLastError(error: libc::size_t);
        fn LoadLibraryW(name: *const libc::c_void) -> *mut libc::c_void;
        fn GetModuleHandleExW(
            dwFlags: libc::c_uint,
            name: *const u16,
            handle: *mut *mut libc::c_void,
        ) -> BOOL;
        fn GetProcAddress(
            handle: *mut libc::c_void,
            name: *const libc::c_char,
        ) -> *mut libc::c_void;
        fn FreeLibrary(handle: *mut libc::c_void);
        fn SetErrorMode(uMode: libc::c_uint) -> libc::c_uint;
    }

    fn win_error_string(err: u32) -> String {
        let mut buffer: [u16; 512] = [0; 512];

        unsafe {
            let len = FormatMessageW(
                FORMAT_MESSAGE_FROM_SYSTEM | FORMAT_MESSAGE_IGNORE_INSERTS,
                std::ptr::null(),
                err,
                0, // language ID (0 = auto)
                buffer.as_mut_ptr(),
                buffer.len() as u32,
                std::ptr::null_mut(),
            );

            if len == 0 {
                return format!("OS Error {}", err);
            }

            // Convert UTF-16 → Rust string
            let msg = OsString::from_wide(&buffer[..len as usize])
                .to_string_lossy()
                .into_owned();

            msg.trim().to_string() // remove extra newline added by Windows
        }
    }
}
