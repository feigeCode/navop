use super::{PersistentEnvironmentValue, merge_persistent_environment};

pub(crate) fn refreshed_windows_environment() -> Vec<(String, String)> {
    let inherited = current_process_environment();
    let system = read_registry_environment(
        windows::Win32::System::Registry::HKEY_LOCAL_MACHINE,
        r"SYSTEM\CurrentControlSet\Control\Session Manager\Environment",
    );
    let user = read_registry_environment(
        windows::Win32::System::Registry::HKEY_CURRENT_USER,
        r"Environment",
    );

    let (system, system_refreshed) = match system {
        Ok(system) => (system, true),
        Err(error) => {
            tracing::warn!("failed to refresh Windows system environment: {error}");
            (Vec::new(), false)
        }
    };
    let (user, user_refreshed) = match user {
        Ok(user) => (user, true),
        Err(error) => {
            tracing::warn!("failed to refresh Windows user environment: {error}");
            (Vec::new(), false)
        }
    };

    if system_refreshed || user_refreshed {
        merge_persistent_environment(system, user, inherited)
    } else {
        inherited
    }
}

fn current_process_environment() -> Vec<(String, String)> {
    std::env::vars_os()
        .map(|(name, value)| {
            (
                name.to_string_lossy().into_owned(),
                value.to_string_lossy().into_owned(),
            )
        })
        .collect()
}

fn read_registry_environment(
    root: windows::Win32::System::Registry::HKEY,
    subkey: &str,
) -> std::io::Result<Vec<PersistentEnvironmentValue>> {
    use std::io;
    use windows::Win32::Foundation::ERROR_SUCCESS;
    use windows::Win32::System::Registry::{KEY_READ, RegCloseKey, RegOpenKeyExW};

    let subkey = subkey.encode_utf16().chain(Some(0)).collect::<Vec<_>>();
    let mut key = windows::Win32::System::Registry::HKEY::default();
    let status = unsafe {
        RegOpenKeyExW(
            root,
            windows::core::PCWSTR::from_raw(subkey.as_ptr()),
            None,
            KEY_READ,
            &mut key,
        )
    };
    if status != ERROR_SUCCESS {
        return Err(io::Error::from_raw_os_error(status.0 as i32));
    }

    let result = enumerate_registry_values(key);
    let close_status = unsafe { RegCloseKey(key) };
    result.and_then(|values| {
        if close_status == ERROR_SUCCESS {
            Ok(values)
        } else {
            Err(io::Error::from_raw_os_error(close_status.0 as i32))
        }
    })
}

fn enumerate_registry_values(
    key: windows::Win32::System::Registry::HKEY,
) -> std::io::Result<Vec<PersistentEnvironmentValue>> {
    use std::io;
    use windows::Win32::Foundation::{ERROR_MORE_DATA, ERROR_NO_MORE_ITEMS, ERROR_SUCCESS};
    use windows::Win32::System::Registry::{REG_EXPAND_SZ, REG_SZ, RegEnumValueW};
    use windows::core::PWSTR;

    let mut values = Vec::new();
    let mut name_buffer = vec![0u16; 256];
    let mut data_buffer = vec![0u8; 4096];
    let mut index = 0;

    loop {
        let mut name_length = name_buffer.len().saturating_sub(1) as u32;
        let mut value_type = REG_SZ.0;
        let mut data_length = data_buffer.len() as u32;
        let status = unsafe {
            RegEnumValueW(
                key,
                index,
                Some(PWSTR::from_raw(name_buffer.as_mut_ptr())),
                &mut name_length,
                None,
                Some(&mut value_type),
                Some(data_buffer.as_mut_ptr()),
                Some(&mut data_length),
            )
        };

        if status == ERROR_NO_MORE_ITEMS {
            break;
        }
        if status == ERROR_MORE_DATA {
            grow_buffer(&mut name_buffer)?;
            grow_buffer(&mut data_buffer)?;
            continue;
        }
        if status != ERROR_SUCCESS {
            return Err(io::Error::from_raw_os_error(status.0 as i32));
        }

        let name = String::from_utf16_lossy(&name_buffer[..name_length as usize]);
        let data_length = data_length as usize;
        if data_length % 2 != 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Windows registry environment string has an odd byte length",
            ));
        }
        let data = data_buffer[..data_length]
            .chunks_exact(2)
            .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
            .take_while(|unit| *unit != 0)
            .collect::<Vec<_>>();
        if value_type == REG_SZ.0 || value_type == REG_EXPAND_SZ.0 {
            values.push(PersistentEnvironmentValue::new(
                &name,
                &String::from_utf16_lossy(&data),
                value_type == REG_EXPAND_SZ.0,
            ));
        }
        index += 1;
    }

    Ok(values)
}

fn grow_buffer<T: Clone + Default>(buffer: &mut Vec<T>) -> std::io::Result<()> {
    let new_length = buffer
        .len()
        .checked_mul(2)
        .filter(|new_length| *new_length > buffer.len())
        .ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "Windows registry environment value is too large",
            )
        })?;
    buffer.resize(new_length, T::default());
    Ok(())
}
