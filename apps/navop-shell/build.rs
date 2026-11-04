include!("../../resources/windows/embed_app_resources.rs");

fn main() {
    embed_windows_resources(
        "../../resources/windows/navop-shell.ico",
        "Navop Shell",
        "Navop Shell - SSH, SFTP and Terminal",
    );
}
