use crate::X11ComponentError;

use super::{check_vulkan_version, decode_vulkan_version};

#[test]
fn decode_packed_driver_version() {
    assert_eq!(decode_vulkan_version((1 << 22) | (3 << 12) | 7), (1, 3, 7));
    assert_eq!(decode_vulkan_version(0), (0, 0, 0));
}

#[test]
fn minimum_vulkan_version_is_enforced() {
    check_vulkan_version("1.3.0", Some("1.3.7".into())).unwrap();
    check_vulkan_version("1.3.0", Some("1.4.0".into())).unwrap();
    for actual in [None, Some("1.2.9".into())] {
        assert!(matches!(
            check_vulkan_version("1.3.0", actual),
            Err(X11ComponentError::VulkanUnavailable { .. })
        ));
    }
}
