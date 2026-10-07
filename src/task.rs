//! `sys task` — query and kill processes (Windows / Ubuntu).
//!
//! Listing enumerates every visible process with its PID, name and
//! executable path; killing accepts a PID or a fuzzy-matched name
//! (substring, case-insensitive — every match is terminated).

use anyhow::{Context, Result, bail};

/// A snapshot of a running process.
struct Proc {
    pid: u32,
    name: String,
    path: Option<String>,
}

fn contains_ci(haystack: &str, needle: &str) -> bool {
    haystack.to_ascii_lowercase().contains(&needle.to_ascii_lowercase())
}

// ---------------------------------------------------------------------------
// Listing
// ---------------------------------------------------------------------------

#[cfg(windows)]
fn list_processes() -> Result<Vec<Proc>> {
    use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
    };

    let snap = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
    if snap == INVALID_HANDLE_VALUE {
        bail!("CreateToolhelp32Snapshot failed: error {}", unsafe { GetLastError() });
    }

    let mut out = Vec::new();
    let mut entry: PROCESSENTRY32W = unsafe { std::mem::zeroed() };
    entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
    let mut ok = unsafe { Process32FirstW(snap, &mut entry) != 0 };
    while ok {
        let pid = entry.th32ProcessID;
        let name = wide_to_string(&entry.szExeFile);
        out.push(Proc {
            pid,
            name,
            path: process_path(pid),
        });
        ok = unsafe { Process32NextW(snap, &mut entry) != 0 };
    }
    unsafe { CloseHandle(snap) };
    Ok(out)
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

#[cfg(not(windows))]
fn list_processes() -> Result<Vec<Proc>> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir("/proc").context("cannot read /proc")? {
        let entry = entry?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.is_empty() || !name.bytes().all(|b| b.is_ascii_digit()) {
            continue;
        }
        let pid: u32 = match name.parse() {
            Ok(p) => p,
            Err(_) => continue,
        };
        let dir = entry.path();
        let comm = match std::fs::read_to_string(dir.join("comm")) {
            Ok(s) => s.trim().to_string(),
            Err(_) => continue,
        };
        if comm.is_empty() {
            continue;
        }
        let path = std::fs::read_link(dir.join("exe"))
            .ok()
            .map(|p| p.to_string_lossy().into_owned());
        out.push(Proc { pid, name: comm, path });
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Killing
// ---------------------------------------------------------------------------

#[cfg(windows)]
fn kill_pid(pid: u32, _force: bool) -> Result<()> {
    use windows_sys::Win32::Foundation::{CloseHandle, GetLastError};
    use windows_sys::Win32::System::Threading::{
        OpenProcess, TerminateProcess, PROCESS_TERMINATE,
    };
    unsafe {
        let h = OpenProcess(PROCESS_TERMINATE, 0, pid);
        if h.is_null() {
            bail!("cannot open process {pid} (access denied or it already exited)");
        }
        let ok = TerminateProcess(h, 1);
        let err = GetLastError();
        CloseHandle(h);
        if ok == 0 {
            bail!("cannot terminate process {pid} (error {err})");
        }
    }
    Ok(())
}

#[cfg(not(windows))]
fn kill_pid(pid: u32, force: bool) -> Result<()> {
    let sig = if force { "-9" } else { "-15" };
    let status = std::process::Command::new("kill")
        .arg(sig)
        .arg(pid.to_string())
        .status()
        .with_context(|| "cannot run `kill`")?;
    if !status.success() {
        bail!("`kill` failed for pid {pid}");
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Port lookup
// ---------------------------------------------------------------------------

/// PIDs of processes using the given port (TCP/UDP, all connection states).
#[cfg(windows)]
fn port_pids(port: u16) -> Result<Vec<u32>> {
    let out = std::process::Command::new("netstat")
        .args(["-ano"])
        .output()
        .with_context(|| "cannot run `netstat`")?;
    if !out.status.success() {
        bail!("`netstat -ano` failed");
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let needle = format!(":{port}");
    let mut pids = Vec::new();
    for line in text.lines() {
        let fields: Vec<&str> = line.split_whitespace().collect();
        if fields.len() < 2 || !fields[1].ends_with(&needle) {
            continue; // fields[1] is the local address
        }
        if let Some(pid) = fields.last().and_then(|s| s.parse::<u32>().ok()) {
            if !pids.contains(&pid) {
                pids.push(pid);
            }
        }
    }
    Ok(pids)
}

/// PIDs of processes listening on the given port.
#[cfg(not(windows))]
fn port_pids(port: u16) -> Result<Vec<u32>> {
    let out = std::process::Command::new("ss")
        .args(["-ltnp"])
        .output()
        .with_context(|| "cannot run `ss`")?;
    if !out.status.success() {
        bail!("`ss -ltnp` failed");
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let needle = format!(":{port}");
    let mut pids = Vec::new();
    for line in text.lines() {
        let fields: Vec<&str> = line.split_whitespace().collect();
        if fields.len() < 4 || !fields[3].ends_with(&needle) {
            continue; // fields[3] is `Local Address:Port`
        }
        // Process column, e.g. users:(("node",pid=1234,fd=10))
        if let Some(proc_field) = fields.get(5).copied() {
            if let Some(pos) = proc_field.find("pid=") {
                let num: String = proc_field[pos + 4..]
                    .chars()
                    .take_while(|c| c.is_ascii_digit())
                    .collect();
                if let Ok(pid) = num.parse::<u32>() {
                    if !pids.contains(&pid) {
                        pids.push(pid);
                    }
                }
            }
        }
    }
    Ok(pids)
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

/// `sys task list [NAME]` — list processes (PID, name, path).
/// NAME fuzzy-matches the process name; a numeric NAME looks up that PID;
/// PORT (with -o/--port) restricts the listing to processes using that port.
pub fn cmd_list(filter: Option<&str>, port: Option<u16>) -> Result<()> {
    let mut procs = list_processes()?;
    procs.sort_by_key(|p| p.pid);

    let pids_on_port: Option<Vec<u32>> = match port {
        Some(p) => Some(port_pids(p)?),
        None => None,
    };
    let port_label = port.map(|p| p.to_string());

    let hits: Vec<&Proc> = procs
        .iter()
        .filter(|p| {
            let on_port = match &pids_on_port {
                Some(ids) => ids.contains(&p.pid),
                None => true,
            };
            if !on_port {
                return false;
            }
            match filter {
                Some(q) => {
                    if let Ok(pid) = q.parse::<u32>() {
                        p.pid == pid
                    } else {
                        contains_ci(&p.name, q)
                    }
                }
                None => true,
            }
        })
        .collect();

    if hits.is_empty() {
        match (filter, &port_label) {
            (Some(q), Some(p)) => bail!("no process matches `{q}` on port {p}"),
            (Some(q), None) => bail!("no process matches `{q}`"),
            (None, Some(p)) => bail!("no process is listening on port {p}"),
            (None, None) => bail!("no processes found"),
        }
    }

    let w_pid = hits
        .iter()
        .map(|p| p.pid.to_string().len())
        .max()
        .unwrap_or(3)
        .max(3);
    let w_name = hits
        .iter()
        .map(|p| p.name.chars().count())
        .max()
        .unwrap_or(4)
        .max(4);
    println!("{:<w_pid$}  {:<w_name$}  {}", "PID", "NAME", "PATH");
    for p in &hits {
        println!(
            "{:<w_pid$}  {:<w_name$}  {}",
            p.pid,
            p.name,
            p.path.as_deref().unwrap_or("")
        );
    }
    match (filter, &port_label) {
        (Some(q), Some(p)) => {
            println!("--- {} of {} processes match `{q}` on port {p}", hits.len(), procs.len());
        }
        (Some(q), None) => println!("--- {} of {} processes match `{q}`", hits.len(), procs.len()),
        (None, Some(p)) => println!("--- {} of {} processes on port {p}", hits.len(), procs.len()),
        (None, None) => println!("--- {} processes", procs.len()),
    }
    Ok(())
}

fn process_name(pid: u32) -> Option<String> {
    list_processes().ok()?.into_iter().find(|p| p.pid == pid).map(|p| p.name)
}

/// `sys task kill TARGET` — kill by PID or by fuzzy-matched name
/// (a name match kills every matching process).
pub fn cmd_kill(target: &str, force: bool) -> Result<()> {
    if let Ok(pid) = target.parse::<u32>() {
        if pid == 0 {
            bail!("pid 0 (system idle) cannot be killed");
        }
        let name = process_name(pid);
        kill_pid(pid, force)?;
        match name {
            Some(n) => println!("killed {pid} ({n})"),
            None => println!("killed {pid}"),
        }
        return Ok(());
    }

    let procs = list_processes()?;
    let hits: Vec<&Proc> = procs.iter().filter(|p| contains_ci(&p.name, target)).collect();
    if hits.is_empty() {
        bail!("no process matches `{target}`");
    }

    let mut killed = 0usize;
    let mut failed = Vec::new();
    for p in &hits {
        match kill_pid(p.pid, force) {
            Ok(()) => {
                println!("killed {} ({})", p.pid, p.name);
                killed += 1;
            }
            Err(e) => failed.push(format!("{} ({}) — {e:#}", p.pid, p.name)),
        }
    }
    if !failed.is_empty() {
        eprintln!("warning: {} process(es) not killed:", failed.len());
        for f in &failed {
            eprintln!("  {f}");
        }
    }
    if killed == 0 {
        bail!("no process could be killed");
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contains_ci_works() {
        assert!(contains_ci("chrome.exe", "CHROME"));
        assert!(contains_ci("chrome.exe", "ome"));
        assert!(!contains_ci("chrome.exe", "edge"));
    }

    #[test]
    fn listing_contains_self() {
        let procs = list_processes().expect("list processes");
        let me = std::process::id();
        assert!(procs.iter().any(|p| p.pid == me), "own pid {me} not listed");
    }

    #[test]
    fn kill_by_pid_fails_for_unknown() {
        // u32::MAX is not a real PID on any supported platform.
        assert!(kill_pid(u32::MAX, false).is_err());
    }

    #[test]
    fn port_pids_finds_listener() {
        use std::net::TcpListener;
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().expect("local addr").port();
        let pids = port_pids(port).unwrap_or_default();
        assert!(
            pids.contains(&std::process::id()),
            "own pid {} not found on port {port}: {pids:?}",
            std::process::id()
        );
    }
}
