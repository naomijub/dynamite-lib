use std::{mem, path::Path};

use dylib::DynamicLibrary;

fn main() {
    #[cfg(target_os = "macos")]
    let path = "demo/target/release/libdemo.dylib";
    #[cfg(target_os = "linux")]
    let path = "demo/target/release/libdemo.so";
    // The math library does not need to be loaded since it is already
    // statically linked in
    let libm = match DynamicLibrary::open(Some(Path::new(path))) {
        Err(error) => panic!("Could not load self as module: {}", error),
        Ok(libm) => libm,
    };

    let cosine_fn: extern "C" fn(libc::c_double) -> libc::c_double = unsafe {
        match libm.symbol("cosine") {
            Err(error) => panic!("Could not load function cos: {}", error),
            Ok(cosine_fn) => mem::transmute::<*mut u8, extern "C" fn(f64) -> f64>(cosine_fn),
        }
    };

    let argument = 0.0;
    let expected_result = 1.0;
    let result = cosine_fn(argument);

    assert_eq!(
        result, expected_result,
        "cosine({}) != {} but equaled {} instead",
        argument, expected_result, result
    )
}
