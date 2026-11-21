use libloading::Library;
use semver::Version;

use crate::X11ComponentError;

type EnumerateLoaderVersion = unsafe extern "system" fn(*mut u32) -> i32;

pub fn decode_vulkan_version(packed: u32) -> (u32, u32, u32) {
    (packed >> 22, (packed >> 12) & 0x3ff, packed & 0xfff)
}

pub fn check_vulkan_version(
    required: &str,
    actual: Option<String>,
) -> Result<(), X11ComponentError> {
    let required_version = Version::parse(required)
        .map_err(|error| X11ComponentError::InvalidManifest(error.to_string()))?;
    let meets = actual
        .as_deref()
        .and_then(|version| Version::parse(version).ok())
        .is_some_and(|version| version >= required_version);
    if meets {
        return Ok(());
    }
    Err(X11ComponentError::VulkanUnavailable {
        required: required.into(),
        actual,
    })
}

/// 直接从 Vulkan loader 查询能力，不依赖 vulkaninfo 可执行文件。
pub fn preflight(required: &str) -> Result<(), X11ComponentError> {
    #[cfg(target_os = "windows")]
    let names = ["vulkan-1.dll"];
    #[cfg(target_os = "linux")]
    let names = ["libvulkan.so.1"];
    #[cfg(not(any(target_os = "windows", target_os = "linux")))]
    let names: [&str; 0] = [];
    for name in names {
        match unsafe { query_loader(name) } {
            Ok(actual) => return check_vulkan_version(required, Some(actual)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => {
                return Err(X11ComponentError::VulkanUnavailable {
                    required: required.into(),
                    actual: Some(error.to_string()),
                });
            }
        }
    }
    check_vulkan_version(required, None)
}

unsafe fn query_loader(name: &str) -> std::io::Result<String> {
    let library =
        unsafe { Library::new(name) }.map_err(|error| std::io::Error::other(error.to_string()))?;
    let enumerate: libloading::Symbol<'_, EnumerateLoaderVersion> =
        unsafe { library.get(b"vkEnumerateInstanceVersion\0") }
            .map_err(|error| std::io::Error::other(error.to_string()))?;
    let mut packed = 0;
    if unsafe { enumerate(&mut packed) } != 0 {
        return Err(std::io::Error::other("vkEnumerateInstanceVersion 调用失败"));
    }
    let (major, minor, patch) = decode_vulkan_version(packed);
    Ok(format!("{major}.{minor}.{patch}"))
}

#[cfg(test)]
mod tests;
