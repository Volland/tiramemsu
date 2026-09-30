// The extension module resolves Python's symbols from the interpreter that loads it
// (`-undefined dynamic_lookup` on macOS), so `cargo build` works without linking libpython.
fn main() {
    pyo3_build_config::add_extension_module_link_args();
}
