fn main() {
    for key in [
        "ONETCLI_EXTENSION_MANIFEST_URL",
        "ONETCLI_EXTENSION_GITHUB_MANIFEST_URL",
    ] {
        println!("cargo:rerun-if-env-changed={key}");
        if let Ok(value) = std::env::var(key)
            && !value.is_empty()
        {
            println!("cargo:rustc-env={key}={value}");
        }
    }
}
