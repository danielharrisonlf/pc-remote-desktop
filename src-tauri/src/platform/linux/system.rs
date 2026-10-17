//! Linux system, XDG directories, POSIX `statvfs`, `0600` secret storage, `xdg-open`, and
//! native file picker implementation of `SystemBackend`.
use crate::{files::SafeName, platform::SystemBackend};
use std::{
    ffi::CString,
    fs::{self, OpenOptions},
    io::{self, Write},
    os::unix::{
        ffi::OsStrExt,
        fs::{OpenOptionsExt, PermissionsExt},
    },
    path::{Path, PathBuf},
    process::Command,
    sync::OnceLock,
    time::{SystemTime, UNIX_EPOCH},
};
use uuid::Uuid;

pub struct LinuxSystem;

/// Parses `/etc/os-release` content into a friendly distro description (e.g. `"Ubuntu 24.04.1 LTS"`).
pub(crate) fn parse_os_release(content: &str) -> Option<String> {
    let mut pretty = None;
    let mut name = None;
    let mut version = None;
    for line in content.lines() {
        let line = line.trim();
        if let Some((k, v)) = line.split_once('=') {
            let unquoted = v.trim().trim_matches('"').trim_matches('\'').trim();
            if unquoted.is_empty() {
                continue;
            }
            match k.trim() {
                "PRETTY_NAME" => pretty = Some(unquoted.to_string()),
                "NAME" => name = Some(unquoted.to_string()),
                "VERSION_ID" => version = Some(unquoted.to_string()),
                _ => {}
            }
        }
    }
    if let Some(p) = pretty {
        return Some(p.chars().take(64).collect());
    }
    match (name, version) {
        (Some(n), Some(v)) => Some(format!("{n} {v}").chars().take(64).collect()),
        (Some(n), None) => Some(n.chars().take(64).collect()),
        _ => None,
    }
}

fn read_os_name() -> Option<String> {
    let content = fs::read_to_string("/etc/os-release")
        .or_else(|_| fs::read_to_string("/usr/lib/os-release"))
        .ok()?;
    parse_os_release(&content)
}

/// Parses `~/.config/user-dirs.dirs` for `XDG_DOWNLOAD_DIR`.
pub(crate) fn parse_xdg_download_dir(content: &str, home: &Path) -> Option<PathBuf> {
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('#') || trimmed.is_empty() {
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("XDG_DOWNLOAD_DIR=") {
            let val = rest.trim().trim_matches('"');
            if let Some(rel) = val.strip_prefix("$HOME/") {
                return Some(home.join(rel));
            } else if val == "$HOME" {
                return Some(home.to_path_buf());
            } else {
                let p = PathBuf::from(val);
                if p.is_absolute() {
                    return Some(p);
                }
            }
        }
    }
    None
}

fn xdg_open(target: &str) -> Result<(), ()> {
    if Command::new("xdg-open")
        .arg(target)
        .status()
        .is_ok_and(|s| s.success())
    {
        return Ok(());
    }
    if Command::new("gio")
        .args(["open", target])
        .status()
        .is_ok_and(|s| s.success())
    {
        return Ok(());
    }
    Err(())
}

/// Checks if a file with `candidate` name (or case-insensitive equivalent) already exists in `folder`.
fn candidate_conflicts(folder: &Path, candidate: &str) -> bool {
    if folder.join(candidate).symlink_metadata().is_ok() {
        return true;
    }
    if let Ok(entries) = fs::read_dir(folder) {
        for entry in entries.flatten() {
            if let Some(existing) = entry.file_name().to_str() {
                if existing.eq_ignore_ascii_case(candidate) {
                    return true;
                }
            }
        }
    }
    false
}

/// Atomically moves `from` to `to` without ever overwriting an existing target.
fn rename_noreplace(from: &Path, to: &Path) -> io::Result<()> {
    let from_c = CString::new(from.as_os_str().as_bytes())
        .map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))?;
    let to_c = CString::new(to.as_os_str().as_bytes())
        .map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))?;
    let rc = unsafe {
        libc::syscall(
            libc::SYS_renameat2,
            libc::AT_FDCWD,
            from_c.as_ptr(),
            libc::AT_FDCWD,
            to_c.as_ptr(),
            libc::RENAME_NOREPLACE,
        )
    };
    if rc == 0 {
        return Ok(());
    }
    let err = io::Error::last_os_error();
    if matches!(err.raw_os_error(), Some(libc::ENOSYS | libc::EINVAL)) {
        // Fallback if filesystem does not support RENAME_NOREPLACE: hard_link never overwrites.
        fs::hard_link(from, to)?;
        let _ = fs::remove_file(from);
        return Ok(());
    }
    Err(err)
}

fn is_virtual_interface(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower == "lo"
        || lower.starts_with("docker")
        || lower.starts_with("veth")
        || lower.starts_with("br-")
        || lower.starts_with("virbr")
        || lower.starts_with("tun")
        || lower.starts_with("tap")
        || lower.starts_with("wg")
        || lower.starts_with("tailscale")
        || lower.starts_with("zt")
}

fn read_mac_addresses_from(sys_net: &Path) -> Vec<String> {
    let Ok(entries) = fs::read_dir(sys_net) else {
        return Vec::new();
    };
    let mut ifaces: Vec<(u8, String, String)> = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if is_virtual_interface(&name) {
            continue;
        }
        let addr_path = entry.path().join("address");
        let Ok(raw) = fs::read_to_string(addr_path) else {
            continue;
        };
        if let Some(mac) = crate::platform::normalize_mac(&raw) {
            let priority = if name.starts_with("en") || name.starts_with("eth") {
                0
            } else if name.starts_with("wl") {
                1
            } else {
                2
            };
            ifaces.push((priority, name, mac));
        }
    }
    ifaces.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
    let mut macs = Vec::new();
    for (_, _, mac) in ifaces {
        if !macs.contains(&mac) {
            macs.push(mac);
        }
    }
    macs.truncate(8);
    macs
}

impl SystemBackend for LinuxSystem {
    fn os_name(&self) -> &'static str {
        static NAME: OnceLock<String> = OnceLock::new();
        NAME.get_or_init(|| read_os_name().unwrap_or_else(|| "Linux".into()))
    }

    fn hostname(&self) -> String {
        if let Ok(h) = fs::read_to_string("/etc/hostname") {
            let trimmed = h.trim();
            if !trimmed.is_empty() {
                return trimmed.to_string();
            }
        }
        std::env::var("HOSTNAME").unwrap_or_else(|_| "PC".into())
    }

    fn mac_addresses(&self) -> Vec<String> {
        read_mac_addresses_from(Path::new("/sys/class/net"))
    }

    fn monotonic_ms(&self) -> u64 {
        let mut ts = libc::timespec {
            tv_sec: 0,
            tv_nsec: 0,
        };
        let rc = unsafe { libc::clock_gettime(libc::CLOCK_BOOTTIME, &mut ts) };
        if rc == 0 && ts.tv_sec >= 0 {
            return (ts.tv_sec as u64)
                .saturating_mul(1000)
                .saturating_add((ts.tv_nsec as u64) / 1_000_000);
        }
        let rc_mono = unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut ts) };
        if rc_mono == 0 && ts.tv_sec >= 0 {
            return (ts.tv_sec as u64)
                .saturating_mul(1000)
                .saturating_add((ts.tv_nsec as u64) / 1_000_000);
        }
        0
    }

    fn local_hms(&self) -> (u16, u16, u16) {
        let secs = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as libc::time_t;
        let mut tm: libc::tm = unsafe { std::mem::zeroed() };
        let ptr = unsafe { libc::localtime_r(&secs, &mut tm) };
        if !ptr.is_null() {
            (
                tm.tm_hour.clamp(0, 23) as u16,
                tm.tm_min.clamp(0, 59) as u16,
                tm.tm_sec.clamp(0, 60) as u16,
            )
        } else {
            (0, 0, 0)
        }
    }

    fn config_dir(&self) -> Result<PathBuf, String> {
        let base = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
            .ok_or("XDG_CONFIG_HOME/HOME unavailable")?;
        let dir = base.join("pc-remote-desktop");
        fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        Ok(dir)
    }

    fn downloads_dir(&self) -> Option<PathBuf> {
        if let Some(dir) = std::env::var_os("XDG_DOWNLOAD_DIR")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
        {
            return Some(dir);
        }
        let home = PathBuf::from(std::env::var_os("HOME")?);
        let config_root = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .unwrap_or_else(|| home.join(".config"));
        if let Ok(content) = fs::read_to_string(config_root.join("user-dirs.dirs")) {
            if let Some(dir) = parse_xdg_download_dir(&content, &home) {
                return Some(dir);
            }
        }
        Some(home.join("Downloads"))
    }

    fn is_plain_directory(&self, meta: &fs::Metadata) -> bool {
        meta.is_dir() && !meta.file_type().is_symlink()
    }

    fn free_disk_bytes(&self, folder: &Path) -> Result<u64, String> {
        let c_path = CString::new(folder.as_os_str().as_bytes()).map_err(|e| e.to_string())?;
        let mut stat: libc::statvfs = unsafe { std::mem::zeroed() };
        let rc = unsafe { libc::statvfs(c_path.as_ptr(), &mut stat) };
        if rc != 0 {
            return Err(io::Error::last_os_error().to_string());
        }
        let bsize = if stat.f_frsize > 0 {
            stat.f_frsize as u64
        } else {
            stat.f_bsize as u64
        };
        Ok((stat.f_bavail as u64).saturating_mul(bsize))
    }

    fn mark_of_the_web(&self, _path: &Path) -> io::Result<()> {
        // Linux filesystems do not use NTFS :Zone.Identifier alternate data streams.
        Ok(())
    }

    fn move_file_unique(
        &self,
        from: &Path,
        folder: &Path,
        name: &SafeName,
        transfer_id: Uuid,
    ) -> Result<String, String> {
        for candidate in name.candidates(transfer_id) {
            if candidate_conflicts(folder, &candidate) {
                continue;
            }
            let target = folder.join(&candidate);
            match rename_noreplace(from, &target) {
                Ok(()) => return Ok(candidate),
                Err(err) if err.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(err) => return Err(err.to_string()),
            }
        }
        Err("nenhum nome livre".into())
    }

    fn secret_file_name(&self) -> &'static str {
        "identity.json"
    }

    fn protect_secret(&self, data: &[u8], _decrypt: bool) -> Result<Vec<u8>, String> {
        Ok(data.to_vec())
    }

    fn save_secret_file(&self, path: &Path, bytes: &[u8]) -> Result<(), String> {
        let temp = path.with_extension("tmp");
        let mut file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&temp)
            .map_err(|e| e.to_string())?;
        file.write_all(bytes).map_err(|e| e.to_string())?;
        file.sync_all().map_err(|e| e.to_string())?;
        drop(file);
        let _ = fs::set_permissions(&temp, fs::Permissions::from_mode(0o600));
        fs::rename(&temp, path).map_err(|e| e.to_string())
    }

    fn open_url(&self, url: &str) -> Result<(), ()> {
        xdg_open(url)
    }

    fn open_youtube_url(&self, url: &str) -> Result<(), ()> {
        xdg_open(url)
    }

    fn open_folder(&self, folder: &Path) -> Result<(), ()> {
        let s = folder.to_str().ok_or(())?;
        xdg_open(s)
    }

    fn pick_files(&self, _owner: isize) -> Result<Vec<PathBuf>, String> {
        // 1. Try zenity (standard on Ubuntu GNOME)
        if let Ok(output) = Command::new("zenity")
            .args([
                "--file-selection",
                "--multiple",
                "--separator=\n",
                "--title=Enviar arquivos ao celular",
            ])
            .output()
        {
            if output.status.success() {
                let text = String::from_utf8_lossy(&output.stdout);
                return Ok(text
                    .lines()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(PathBuf::from)
                    .collect());
            }
            // Exit code 1 means the user cancelled the dialog.
            if output.status.code() == Some(1) {
                return Ok(Vec::new());
            }
        }

        // 2. Fallback to kdialog (KDE Plasma)
        if let Ok(output) = Command::new("kdialog")
            .args([
                "--getopenfilename",
                ".",
                "--multiple",
                "--separate-output",
                "--title",
                "Enviar arquivos ao celular",
            ])
            .output()
        {
            if output.status.success() {
                let text = String::from_utf8_lossy(&output.stdout);
                return Ok(text
                    .lines()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(PathBuf::from)
                    .collect());
            }
            if output.status.code() == Some(1) {
                return Ok(Vec::new());
            }
        }

        Err(
            "Não foi possível abrir o seletor de arquivos (instale o pacote 'zenity' ou arraste os arquivos para a janela)."
                .into(),
        )
    }

    fn exit_after_update_install(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_ubuntu_os_release_and_xdg_user_dirs() {
        let sample = "NAME=\"Ubuntu\"\nVERSION_ID=\"24.04\"\nPRETTY_NAME=\"Ubuntu 24.04.1 LTS\"\n";
        assert_eq!(
            parse_os_release(sample).as_deref(),
            Some("Ubuntu 24.04.1 LTS")
        );

        let dirs = "# written by xdg-user-dirs-update\nXDG_DOWNLOAD_DIR=\"$HOME/Downloads\"\n";
        assert_eq!(
            parse_xdg_download_dir(dirs, Path::new("/home/ubuntu")),
            Some(PathBuf::from("/home/ubuntu/Downloads"))
        );
    }

    #[test]
    fn this_pc_reports_a_short_name() {
        let name = LinuxSystem.os_name();
        assert!(!name.is_empty() && name.len() <= 64, "{name}");
    }
}
