include!("../../resources/windows/embed_app_resources.rs");

fn main() {
    embed_windows_resources(
        "../../resources/windows/navop.ico",
        "Navop Data",
        "Navop Data - Database Management",
    );
}
