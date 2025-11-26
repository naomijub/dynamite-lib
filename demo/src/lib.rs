#[unsafe(no_mangle)]
pub extern "C" fn cos(x: libc::c_double) -> libc::c_double {
    println!("cost");
    let x = x.cos();
    println!("{x}");
    x
}