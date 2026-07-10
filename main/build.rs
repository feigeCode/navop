fn main() {
    #[cfg(target_os = "windows")]
    {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("../resources/windows/onetcli.ico");
        res.set("ProductName", "Navop");
        res.set("FileDescription", "Navop - Database and Remote Operations");
        res.set("LegalCopyright", "Copyright (c) 2025 Navop");
        res.compile().expect("Failed to compile Windows resources");
    }
}
