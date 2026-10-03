//! Windows registry backend.
//!
//! User scope  : HKEY_CURRENT_USER\Environment
//! Machine scope: HKEY_LOCAL_MACHINE\SYSTEM\CurrentControlSet\Control\Session Manager\Environment

use crate::store::Scope;
use anyhow::{Context, Result, bail};
use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ, KEY_WRITE, RegType};
use winreg::{RegKey, RegValue};

const USER_KEY: &str = r"Environment";
const MACHINE_KEY: &str = r"SYSTEM\CurrentControlSet\Control\Session Manager\Environment";
const PATH_VALUE: &str = "Path";

fn open_key(scope: Scope, write: bool) -> Result<RegKey> {
    let (predef, path) = match scope {
        Scope::User => (HKEY_CURRENT_USER, USER_KEY),
        Scope::Machine => (HKEY_LOCAL_MACHINE, MACHINE_KEY),
    };
    let root = RegKey::predef(predef);
    let flags = if write { KEY_READ | KEY_WRITE } else { KEY_READ };
    let key = root
        .open_subkey_with_flags(path, flags)
        .with_context(|| format!("cannot open registry key `{path}` (scope {})", scope.as_str()))?;
    Ok(key)
}

fn read_string(key: &RegKey, name: &str) -> Option<String> {
    // winreg's String FromRegValue handles REG_SZ and REG_EXPAND_SZ.
    key.get_value::<String, _>(name).ok()
}

fn write_string(key: &RegKey, name: &str, value: &str) -> Result<()> {
    // Preserve REG_EXPAND_SZ semantics when the value references other variables.
    let vtype = if value.contains('%') {
        RegType::REG_EXPAND_SZ
    } else {
        RegType::REG_SZ
    };
    let mut bytes: Vec<u8> = value.encode_utf16().flat_map(|u| u.to_le_bytes()).collect();
    bytes.extend_from_slice(&[0, 0]); // NULL terminator
    key.set_raw_value(name, &RegValue { bytes, vtype })
        .with_context(|| format!("failed to write registry value `{name}`"))
}

pub fn get_path(scope: Scope) -> Option<String> {
    open_key(scope, false).ok().and_then(|k| read_string(&k, PATH_VALUE))
}

pub fn set_path(scope: Scope, value: &str) -> Result<()> {
    let key = open_key(scope, true).map_err(|e| {
        if scope == Scope::Machine {
            anyhow::anyhow!(
                "{e:#}\nhint: modifying the machine PATH requires administrator rights; run the terminal as Administrator or use --scope user"
            )
        } else {
            e
        }
    })?;
    write_string(&key, PATH_VALUE, value)
}

pub fn get_var(scope: Scope, name: &str) -> Option<String> {
    open_key(scope, false).ok().and_then(|k| read_string(&k, name))
}

pub fn set_var(scope: Scope, name: &str, value: &str) -> Result<()> {
    if name.is_empty() {
        bail!("variable name must not be empty");
    }
    let key = open_key(scope, true).map_err(|e| {
        if scope == Scope::Machine {
            anyhow::anyhow!(
                "{e:#}\nhint: modifying machine environment requires administrator rights; run the terminal as Administrator or use --scope user"
            )
        } else {
            e
        }
    })?;
    write_string(&key, name, value)
}

pub fn unset_var(scope: Scope, name: &str) -> Result<()> {
    let key = open_key(scope, true).map_err(|e| {
        if scope == Scope::Machine {
            anyhow::anyhow!(
                "{e:#}\nhint: modifying machine environment requires administrator rights; run the terminal as Administrator or use --scope user"
            )
        } else {
            e
        }
    })?;
    match key.delete_value(name) {
        Ok(_) => Ok(()),
        Err(e) => {
            if e.raw_os_error() == Some(2) {
                // ERROR_FILE_NOT_FOUND: value already absent.
                Ok(())
            } else {
                Err(e).with_context(|| format!("failed to delete registry value `{name}`"))
            }
        }
    }
}

/// Notify the system (explorer, other processes) that the environment changed.
pub fn broadcast() {
    use windows_sys::Win32::Foundation::HWND;
    use windows_sys::Win32::UI::WindowsAndMessaging::{SendMessageTimeoutW, SMTO_ABORTIFHUNG};
    const WM_SETTINGCHANGE: u32 = 0x001A;
    // HWND_BROADCAST
    let hwnd = (-1isize as isize) as HWND;
    let wide: Vec<u16> = "Environment".encode_utf16().chain(std::iter::once(0)).collect();
    unsafe {
        SendMessageTimeoutW(
            hwnd,
            WM_SETTINGCHANGE,
            0,
            wide.as_ptr() as isize,
            SMTO_ABORTIFHUNG,
            5000,
            std::ptr::null_mut(),
        );
    }
}
