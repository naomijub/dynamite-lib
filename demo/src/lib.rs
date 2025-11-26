#[unsafe(no_mangle)]
pub extern "C" fn cosine(x: f64) -> f64 {
    f64::cos(x)
}
