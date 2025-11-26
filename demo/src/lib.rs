#[unsafe(no_mangle)]
pub extern "C" fn cos(x: libc::c_double) -> libc::c_double {
    let x = x.cos();
    x as libc::c_double
}
