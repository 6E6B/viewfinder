fn main() {
    println!("cargo:rerun-if-changed=data/icons");
    println!("cargo:rerun-if-changed=data/icons.gresource.xml");
    let output =
        std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap()).join("icons.gresource");
    let status = std::process::Command::new("glib-compile-resources")
        .args(["--sourcedir=data", "data/icons.gresource.xml", "--target"])
        .arg(output)
        .status()
        .expect("glib-compile-resources is required to bundle application icons");
    assert!(status.success(), "Could not compile application icons");
}
