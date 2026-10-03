//! `sysenv link` — make a file executable from anywhere by placing a link
//! (hard link / symlink / copy, or a `.cmd` shim on Windows) into a directory
//! that is on the PATH.

use crate::store::common;
use crate::store::{Scope, current_path_entries};
use crate::path;
use anyhow::{Context, Result, bail};
use std::path::{Path, PathBuf};

#[derive(clap::ValueEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkMethod {
    Hard,
    Symlink,
    Copy,
}

/// Default managed bin directory (added to user PATH automatically).
pub(crate) fn default_bin_dir() -> Result<PathBuf> {
    #[cfg(windows)]
    {
        let user = std::env::var("USERPROFILE").context("USERPROFILE is not set")?;
        Ok(PathBuf::from(user).join(".sysenv").join("bin"))
    }
    #[cfg(not(windows))]
    {
        let home = std::env::var("HOME").context("HOME is not set")?;
        Ok(PathBuf::from(&home).join(".local").join("bin"))
    }
}

fn system_bin_dir() -> Result<PathBuf> {
    #[cfg(windows)]
    {
        let sysroot = std::env::var("SystemRoot").unwrap_or_else(|_| "C:\\Windows".to_string());
        Ok(PathBuf::from(sysroot).join("System32"))
    }
    #[cfg(not(windows))]
    {
        Ok(PathBuf::from("/usr/local/bin"))
    }
}

/// Directly linkable executable extensions on Windows (others get a .cmd shim).
#[cfg(windows)]
fn is_direct_linkable(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    [".exe", ".com", ".bat", ".cmd"]
        .iter()
        .any(|ext| lower.ends_with(ext))
}

pub fn cmd_link(
    file: &str,
    dir: Option<PathBuf>,
    system: bool,
    method: Option<LinkMethod>,
    name: Option<String>,
    force: bool,
    temporary: bool,
    #[allow(unused)]
    no_shim: bool,
) -> Result<()> {
    // 1. Resolve the target file.
    let target = std::path::absolute(file)
        .with_context(|| format!("cannot resolve path `{file}`"))?;
    if !target.is_file() {
        bail!("`{}` is not a regular file", target.display());
    }
    let target_name = target
        .file_name()
        .and_then(|n| n.to_str())
        .context("target file name is not valid UTF-8")?;

    // 2. Choose the bin directory.
    let bin_dir = if system {
        system_bin_dir()?
    } else if let Some(d) = dir {
        d
    } else {
        default_bin_dir()?
    };
    std::fs::create_dir_all(&bin_dir)
        .with_context(|| format!("cannot create directory {}", bin_dir.display()))?;

    // 3. Make sure the bin directory is on the PATH (unless --temporary).
    let bin_abs = common::absolutize_dir(&bin_dir.to_string_lossy())?;
    let on_path = current_path_entries().iter().any(|e| common::path_eq(e, &bin_abs));
    if !on_path {
        if temporary {
            println!(
                "[temporary] {} is not on PATH yet; add it with:\n  {}",
                bin_abs,
                common::shell_export_snippet("PATH", &common::join_path(&common::merge_entries(&current_path_entries(), &[bin_abs.clone()])))
            );
        } else {
            path::cmd_add(&bin_abs, Scope::User, false, false)?;
        }
    }

    // 4. Compute the link name and whether a shim is needed.
    let link_name = match name {
        Some(n) => n,
        None => target_name.to_string(),
    };

    #[cfg(windows)]
    let (link_name, shim_needed) = if no_shim {
        (link_name, false)
    } else if is_direct_linkable(&link_name) {
        (link_name, false)
    } else if link_name.to_ascii_lowercase().ends_with(".cmd")
        || link_name.to_ascii_lowercase().ends_with(".bat")
    {
        (link_name, false)
    } else {
        (format!("{link_name}.cmd"), true)
    };

    #[cfg(not(windows))]
    let _shim_needed = false;

    let dest = bin_dir.join(&link_name);

    if dest.exists() {
        if !force {
            bail!(
                "{} already exists (use --force to overwrite)",
                dest.display()
            );
        }
        std::fs::remove_file(&dest)
            .with_context(|| format!("cannot remove existing {}", dest.display()))?;
    }

    // 5. Create the link.
    #[cfg(windows)]
    if shim_needed {
        write_cmd_shim(&dest, &target)?;
        println!(
            "Created shim {} -> {}\nThe command is now available as `{}` from any directory.",
            dest.display(),
            target.display(),
            link_name.trim_end_matches(".cmd")
        );
        return Ok(());
    }

    let m = method.unwrap_or(LinkMethod::Hard);
    match create_link(m, &target, &dest) {
        Ok(kind) => {
            #[cfg(not(windows))]
            ensure_executable(&target, &dest, kind == "copy");
            println!(
                "Created {kind} link {} -> {}\nThe command is now available as `{}` from any directory.",
                dest.display(),
                target.display(),
                link_name
            );
        }
        Err(e) => {
            // Fall back through auto chain: hard -> symlink -> copy.
            let fallbacks: &[LinkMethod] = if m == LinkMethod::Hard {
                &[LinkMethod::Symlink, LinkMethod::Copy]
            } else if m == LinkMethod::Symlink {
                &[LinkMethod::Copy]
            } else {
                &[]
            };
            let mut last_err = e;
            let mut created = None;
            for next in fallbacks {
                match create_link(*next, &target, &dest) {
                    Ok(kind) => {
                        created = Some(kind);
                        break;
                    }
                    Err(e2) => last_err = e2,
                }
            }
            match created {
                Some(kind) => {
                    #[cfg(not(windows))]
                    ensure_executable(&target, &dest, kind == "copy");
                    println!(
                        "Created {kind} link {} -> {} (fallback after hard-link failure).\nThe command is now available as `{}` from any directory.",
                        dest.display(),
                        target.display(),
                        link_name
                    );
                }
                None => {
                    bail!(
                        "failed to create a link to {}:\n  {last_err:#}\nhint: use --method copy if the target is on a different volume than {}",
                        target.display(),
                        bin_dir.display()
                    )
                }
            }
        }
    }
    Ok(())
}

fn create_link(method: LinkMethod, target: &Path, dest: &Path) -> Result<&'static str> {
    match method {
        LinkMethod::Hard => {
            std::fs::hard_link(target, dest)
                .with_context(|| format!("hard link {} -> {}", dest.display(), target.display()))?;
            Ok("hard")
        }
        LinkMethod::Symlink => {
            #[cfg(windows)]
            {
                std::os::windows::fs::symlink_file(target, dest).with_context(|| {
                    format!(
                        "symlink {} -> {} (may require Developer Mode or admin rights)",
                        dest.display(),
                        target.display()
                    )
                })?;
            }
            #[cfg(not(windows))]
            {
                std::os::unix::fs::symlink(target, dest).with_context(|| {
                    format!("symlink {} -> {}", dest.display(), target.display())
                })?;
            }
            Ok("symlink")
        }
        LinkMethod::Copy => {
            std::fs::copy(target, dest)
                .with_context(|| format!("copy {} -> {}", target.display(), dest.display()))?;
            Ok("copy")
        }
    }
}

#[cfg(not(windows))]
fn ensure_executable(target: &Path, dest: &Path, is_copy: bool) {
    use std::os::unix::fs::PermissionsExt;
    let the_file = if is_copy { dest } else { target };
    if let Ok(meta) = std::fs::metadata(the_file) {
        let mut perms = meta.permissions();
        if perms.mode() & 0o111 == 0 {
            let new_mode = perms.mode() | 0o755;
            perms.set_mode(new_mode);
            let _ = std::fs::set_permissions(the_file, perms);
            println!("Made {} executable.", the_file.display());
        }
    }
}

#[cfg(windows)]
fn write_cmd_shim(dest: &Path, target: &Path) -> Result<()> {
    let target_quoted = format!("\"{}\"", target.display());
    let content = format!("@echo off\r\n{target_quoted} %*\r\n");
    std::fs::write(dest, content)
        .with_context(|| format!("cannot write shim {}", dest.display()))
}
