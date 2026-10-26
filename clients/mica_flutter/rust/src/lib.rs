pub mod api;
mod frb_generated;

/// `<16 hex digits>\n<root>;<root>…` — the source fingerprint `build.rs`
/// computed. A plain C symbol, not an FRB function, so the staleness check
/// does not depend on the generated bindings it is meant to vouch for.
#[no_mangle]
pub extern "C" fn mica_ffi_source_fingerprint() -> *const std::ffi::c_char {
    concat!(
        env!("MICA_FFI_SOURCE_HASH"),
        "\n",
        env!("MICA_FFI_SOURCE_ROOTS"),
        "\0"
    )
    .as_ptr()
    .cast()
}
