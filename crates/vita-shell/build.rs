fn main() {
    println!("cargo:rerun-if-changed=vita-shell.manifest");
    if std::env::var_os("CARGO_CFG_WINDOWS").is_some() {
        let mut res = winres::WindowsResource::new();
        res.set_manifest_file("vita-shell.manifest");
        res.compile().expect("Failed to compile Windows manifest resource");
    }
}
