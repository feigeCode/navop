#[cfg(windows_rdp_probe_native)]
unsafe extern "C" {
    fn windows_rdp_probe_run() -> i32;
}

#[cfg(windows_rdp_probe_native)]
fn main() {
    let code = unsafe { windows_rdp_probe_run() };
    std::process::exit(code);
}

#[cfg(not(windows_rdp_probe_native))]
fn main() {
    println!("windows-rdp-probe status=unsupported reason=requires-windows-msvc-atl");
}
