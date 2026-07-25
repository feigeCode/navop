use std::path::Path;

const BUILD_TIME_ENV_KEYS: &[&str] = &[
    "SUPABASE_URL",
    "SUPABASE_ANON_KEY",
    "NAVOP_PUBLIC_BASE_URL",
    "NAVOP_WEBSITE_BASE_URL",
    "NAVOP_UPDATE_URL",
    "NAVOP_UPDATE_DOWNLOAD_URL",
];

fn main() {
    if std::env::var("PROFILE").as_deref() == Ok("debug") {
        load_workspace_env_files();
    }

    // 环境变量或本地环境文件变化后，重新运行 build script 并刷新 option_env!。
    for key in BUILD_TIME_ENV_KEYS {
        println!("cargo:rerun-if-env-changed={key}");
        if let Ok(val) = std::env::var(key)
            && !val.is_empty()
        {
            println!("cargo:rustc-env={key}={val}");
        }
    }
}

fn load_workspace_env_files() {
    let workspace_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");

    // dotenvy 默认不覆盖调用 cargo 时已经存在的环境变量，因此优先级为：
    // shell 环境变量 > .env.local > .env。
    for file_name in [".env.local", ".env"] {
        let path = workspace_dir.join(file_name);
        println!("cargo:rerun-if-changed={}", path.display());
        if path.is_file()
            && let Err(error) = dotenvy::from_path(&path)
        {
            println!(
                "cargo:warning=无法加载构建环境文件 {}: {error}",
                path.display()
            );
        }
    }
}
