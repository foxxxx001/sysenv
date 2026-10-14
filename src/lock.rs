//! `sys path unlock` — check whether a file or directory is locked (held
//! open) by other processes, list the lockers, and release the lock (Windows).
//!
//! Detection and release use the Windows Restart Manager API (Rstrtmgr.dll):
//! every process holding the path open is reported with its PID / name /
//! executable path, then those processes are shut down so their handles close
//! and the path is freed.

use anyhow::{Context, Result, bail};

/// `ERROR_MORE_DATA` — the probe buffer was too small, the real size is returned.
const ERROR_MORE_DATA: u32 = 234;

/// One process holding the target path open.
#[derive(Debug)]
struct Locker {
    pid: u32,
    name: String,
    path: Option<String>,
}

/// `sys path unlock TARGET` — check / unlock (Windows only).
pub fn cmd_path_unlock(target: &str) -> Result<()> {
    #[cfg(not(windows))]
    {
        let _ = target;
        bail!("`path unlock` is Windows-only (Linux file locks are advisory and need no unlock)")
    }

    #[cfg(windows)]
    cmd_path_unlock_win(target)
}

#[cfg(windows)]
fn cmd_path_unlock_win(target: &str) -> Result<()> {
    use windows_sys::Win32::System::RestartManager::RmForceShutdown;
    use windows_sys::Win32::System::RestartManager::RmShutdown;

    let abs = absolutize(target)?;
    let path_str = abs.to_string_lossy();

    let lockers = detect_lockers(&path_str)?;
    if lockers.is_empty() {
        println!("not locked: {path_str} (no process has it open)");
        return Ok(());
    }
    print_lockers("locked by", &lockers);

    // Release the lock by shutting down the processes holding it open.
    with_rm_session(|session| {
        register(session, &path_str)?;
        let err = unsafe { RmShutdown(session, RmForceShutdown as u32, None) };
        if err != 0 {
            bail!("unlock (RmShutdown) failed: error {err}");
        }
        Ok(())
    })?;

    // The shut-down session's own list stays stale until it ends, so re-check
    // in fresh sessions while the processes finish exiting (termination is
    // asynchronous; the loop only runs long when the lock persists).
    let mut remaining = lockers;
    for _ in 0..20 {
        std::thread::sleep(std::time::Duration::from_millis(300));
        remaining = detect_lockers(&path_str)?;
        if remaining.is_empty() {
            break;
        }
    }
    if remaining.is_empty() {
        println!("unlocked: {path_str}");
        Ok(())
    } else {
        print_lockers("still locked by", &remaining);
        bail!(
            "unlock incomplete: {} process(es) still hold the path",
            remaining.len()
        );
    }
}

/// Resolve `target` against the current directory; the path must exist.
#[cfg(windows)]
fn absolutize(target: &str) -> Result<std::path::PathBuf> {
    let path = std::path::Path::new(target);
    let abs = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    };
    std::fs::metadata(&abs)
        .with_context(|| format!("`{target}` does not exist or is not accessible"))?;
    Ok(abs)
}

/// Run `f` with a fresh Restart Manager session, always ending it afterwards.
#[cfg(windows)]
fn with_rm_session<T>(f: impl FnOnce(u32) -> Result<T>) -> Result<T> {
    use windows_sys::Win32::System::RestartManager::{
        CCH_RM_SESSION_KEY, RmEndSession, RmStartSession,
    };

    let mut session: u32 = 0;
    let mut key = [0u16; (CCH_RM_SESSION_KEY + 1) as usize];
    let err = unsafe { RmStartSession(&mut session, 0, key.as_mut_ptr()) };
    if err != 0 {
        bail!("RmStartSession failed: error {err}");
    }
    let result = f(session);
    let _ = unsafe { RmEndSession(session) };
    result
}

/// Every process currently holding `path` open, from a fresh session.
#[cfg(windows)]
fn detect_lockers(path: &str) -> Result<Vec<Locker>> {
    with_rm_session(|session| {
        register(session, path)?;
        query_lockers(session)
    })
}

/// Register `path` as the session's resource (the only resource).
#[cfg(windows)]
fn register(session: u32, path: &str) -> Result<()> {
    use windows_sys::Win32::System::RestartManager::RmRegisterResources;

    let w: Vec<u16> = path.encode_utf16().chain(std::iter::once(0)).collect();
    let err = unsafe {
        RmRegisterResources(session, 1, &w.as_ptr(), 0, std::ptr::null(), 0, std::ptr::null())
    };
    if err != 0 {
        bail!("cannot register `{path}` with Restart Manager: error {err}");
    }
    Ok(())
}

/// Every process currently holding the session's registered resource open.
#[cfg(windows)]
fn query_lockers(session: u32) -> Result<Vec<Locker>> {
    use windows_sys::Win32::System::RestartManager::{RmGetList, RM_PROCESS_INFO};

    // Probe: NULL buffer returns ERROR_MORE_DATA and the required size.
    let mut needed: u32 = 0;
    let mut count: u32 = 0;
    let mut reboot: u32 = 0;
    let err = unsafe {
        RmGetList(session, &mut needed, &mut count, std::ptr::null_mut(), &mut reboot)
    };
    if err != 0 && err != ERROR_MORE_DATA {
        bail!("RmGetList failed: error {err}");
    }
    if needed == 0 {
        return Ok(Vec::new());
    }

    let mut infos: Vec<RM_PROCESS_INFO> = vec![unsafe { std::mem::zeroed() }; needed as usize];
    let mut got = needed;
    let err = unsafe { RmGetList(session, &mut needed, &mut got, infos.as_mut_ptr(), &mut reboot) };
    if err != 0 {
        bail!("RmGetList failed: error {err}");
    }

    let mut out = Vec::with_capacity(got as usize);
    for info in infos.iter().take(got as usize) {
        let pid = info.Process.dwProcessId;
        out.push(Locker {
            pid,
            name: wide_to_string(&info.strAppName),
            path: process_path(pid),
        });
    }
    Ok(out)
}

#[cfg(windows)]
fn print_lockers(label: &str, lockers: &[Locker]) {
    let w_pid = lockers
        .iter()
        .map(|l| l.pid.to_string().len())
        .max()
        .unwrap_or(3)
        .max(3);
    let w_name = lockers
        .iter()
        .map(|l| l.name.chars().count())
        .max()
        .unwrap_or(4)
        .max(4);
    println!("{label} {} process(es):", lockers.len());
    println!("{:<w_pid$}  {:<w_name$}  {}", "PID", "NAME", "PATH");
    for l in lockers {
        println!(
            "{:<w_pid$}  {:<w_name$}  {}",
            l.pid,
            l.name,
            l.path.as_deref().unwrap_or("")
        );
    }
}

#[cfg(windows)]
fn wide_to_string(w: &[u16]) -> String {
    let len = w.iter().position(|&c| c == 0).unwrap_or(w.len());
    String::from_utf16_lossy(&w[..len])
}

#[cfg(windows)]
fn process_path(pid: u32) -> Option<String> {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::{
        OpenProcess, QueryFullProcessImageNameW, PROCESS_QUERY_LIMITED_INFORMATION,
    };
    unsafe {
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if h.is_null() {
            return None;
        }
        let mut buf = [0u16; 32768];
        let mut len = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(h, 0, buf.as_mut_ptr(), &mut len) != 0;
        CloseHandle(h);
        if ok {
            Some(String::from_utf16_lossy(&buf[..len as usize]))
        } else {
            None
        }
    }
}

// ---------------------------------------------------------------------------
// Tests (Windows only: locking semantics are platform-specific)
// ---------------------------------------------------------------------------

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, GENERIC_READ};
    use windows_sys::Win32::Storage::FileSystem::{CreateFileW, OPEN_EXISTING};

    fn temp_file(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("sys-lock-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join(name);
        std::fs::write(&f, b"x").unwrap();
        f
    }

    /// Open `path` with sharing denied, so the file is locked by this process.
    fn locked_handle(path: &std::path::Path) -> *mut core::ffi::c_void {
        let w: Vec<u16> = path
            .to_string_lossy()
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        let h = unsafe {
            CreateFileW(
                w.as_ptr(),
                GENERIC_READ,
                0, // no sharing: locks the file
                std::ptr::null(),
                OPEN_EXISTING,
                0,
                std::ptr::null_mut(),
            )
        };
        assert!(
            !h.is_null(),
            "CreateFileW failed: error {}",
            unsafe { GetLastError() }
        );
        h
    }

    fn detect(path: &std::path::Path) -> Vec<Locker> {
        let s = absolutize(path.to_str().unwrap()).unwrap();
        detect_lockers(&s.to_string_lossy()).expect("restart manager session")
    }

    #[test]
    fn no_handle_is_not_locked() {
        let f = temp_file("free.txt");
        assert!(detect(&f).is_empty());
    }

    #[test]
    fn open_handle_is_reported() {
        let f = temp_file("held.txt");
        let h = locked_handle(&f);
        let lockers = detect(&f);
        assert!(
            lockers.iter().any(|l| l.pid == std::process::id()),
            "own pid {} not reported as locker: {lockers:?}",
            std::process::id()
        );
        unsafe { CloseHandle(h) };
        // Once the handle is closed the path is free again.
        assert!(detect(&f).is_empty());
    }
}
