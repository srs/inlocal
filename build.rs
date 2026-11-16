// rust-embed needs ui/dist to exist at compile time. Create a placeholder so
// `cargo build` works before the UI has been built.
fn main() {
    let dist = std::path::Path::new("ui/dist");
    if !dist.join("index.html").exists() {
        std::fs::create_dir_all(dist).expect("create ui/dist");
        std::fs::write(
            dist.join("index.html"),
            "<!doctype html><title>inlocal</title><p>UI not built. Run <code>bun install && bun run build</code> in <code>ui/</code>, then rebuild.</p>",
        )
        .expect("write placeholder");
    }
    println!("cargo:rerun-if-changed=ui/dist");
}
