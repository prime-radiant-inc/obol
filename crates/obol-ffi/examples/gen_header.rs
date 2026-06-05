//! Regenerate the committed C header. Run via `scripts/gen-header.sh`.
fn main() {
    let crate_dir = env!("CARGO_MANIFEST_DIR");
    let config = cbindgen::Config::from_file(format!("{crate_dir}/cbindgen.toml")).unwrap();
    let bindings = cbindgen::Builder::new()
        .with_crate(crate_dir)
        .with_config(config)
        .generate()
        .expect("cbindgen generation failed");
    let mut buf: Vec<u8> = Vec::new();
    bindings.write(&mut buf);
    let out = format!("{crate_dir}/include/obol.h");
    std::fs::create_dir_all(format!("{crate_dir}/include")).unwrap();
    std::fs::write(&out, &buf).unwrap();
    println!("wrote {out}");
}
