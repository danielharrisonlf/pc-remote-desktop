//! Windows system, filesystem, DPAPI secret storage, shell, and file picker implementation of `SystemBackend`.
use crate::{files::SafeName, platform::SystemBackend};
use std::{
    fs::{self, OpenOptions},
    io::{self, Write},
    os::windows::{ffi::OsStringExt, fs::MetadataExt},
    path::{Path, PathBuf},
    sync::OnceLock,
};
use uuid::Uuid;
use windows::{
    core::{w, HSTRING, PCWSTR},
    Win32::{
        Foundation::{
            LocalFree, ERROR_ALREADY_EXISTS, ERROR_CANCELLED, ERROR_FILE_EXISTS, HLOCAL, HWND,
        },
        Security::Cryptography::{
            CryptProtectData, CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
        },
        Storage::FileSystem::{
            GetDiskFreeSpaceExW, MoveFileExW, FILE_ATTRIBUTE_REPARSE_POINT, MOVEFILE_WRITE_THROUGH,
        },
        System::{
            Com::{
                CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize,
                CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE,
            },
            SystemInformation::{GetLocalTime, GetTickCount64},
        },
        UI::{
            Shell::{
                FOLDERID_Downloads, FileOpenDialog, IFileOpenDialog, SHGetKnownFolderPath,
                ShellExecuteW, FOS_ALLOWMULTISELECT, FOS_FILEMUSTEXIST, FOS_FORCEFILESYSTEM,
                KF_FLAG_DEFAULT, SIGDN_FILESYSPATH,
            },
            WindowsAndMessaging::SW_SHOWNORMAL,
        },
    },
};

pub struct WindowsSystem;

const MARK_OF_THE_WEB: &[u8] = b"[ZoneTransfer]\r\nZoneId=3\r\n";

fn read_os_name() -> Option<String> {
    let key = windows_registry::LOCAL_MACHINE
        .open("SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion")
        .ok()?;
    let build = key.get_string("CurrentBuildNumber").ok()?;
    let edition = key.get_string("EditionID").unwrap_or_default();
    Some(describe_windows(&build, &edition))
}

/// ProductName still says "Windows 10" on Windows 11, so the build decides.
pub(crate) fn describe_windows(build: &str, edition: &str) -> String {
    let version = match build.trim().parse::<u32>() {
        Ok(build) if build >= 22_000 => "Windows 11",
        Ok(build) if build >= 10_240 => "Windows 10",
        _ => "Windows",
    };
    let edition = match edition.trim() {
        "Core" | "CoreSingleLanguage" | "CoreCountrySpecific" => " Home",
        "Professional" | "ProfessionalWorkstation" => " Pro",
        "Enterprise" | "EnterpriseS" => " Enterprise",
        "Education" => " Education",
        _ => "",
    };
    format!("{version}{edition}")
}

fn shell_open(target: &HSTRING) -> Result<(), ()> {
    let operation = HSTRING::from("open");
    // ShellExecute returns a value greater than 32 on success.
    let result = unsafe {
        ShellExecuteW(
            None,
            &operation,
            target,
            PCWSTR::null(),
            PCWSTR::null(),
            SW_SHOWNORMAL,
        )
    };
    if result.0 as isize > 32 {
        Ok(())
    } else {
        Err(())
    }
}

unsafe fn show_file_dialog(owner: isize) -> windows::core::Result<Vec<PathBuf>> {
    let dialog: IFileOpenDialog =
        unsafe { CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER)? };
    let options = unsafe { dialog.GetOptions()? };
    unsafe {
        dialog
            .SetOptions(options | FOS_ALLOWMULTISELECT | FOS_FORCEFILESYSTEM | FOS_FILEMUSTEXIST)?;
        dialog.SetTitle(&HSTRING::from("Enviar arquivos ao celular"))?;
        dialog.SetOkButtonLabel(w!("Enviar"))?;
    }
    let parent = (owner != 0).then(|| HWND(owner as *mut _));
    if let Err(error) = unsafe { dialog.Show(parent) } {
        return if error.code() == ERROR_CANCELLED.to_hresult() {
            Ok(Vec::new())
        } else {
            Err(error)
        };
    }
    let items = unsafe { dialog.GetResults()? };
    let mut paths = Vec::new();
    for index in 0..unsafe { items.GetCount()? } {
        let name = unsafe { items.GetItemAt(index)?.GetDisplayName(SIGDN_FILESYSPATH)? };
        let path = unsafe { name.to_string() };
        unsafe { CoTaskMemFree(Some(name.0 as *const _)) };
        if let Ok(path) = path {
            paths.push(PathBuf::from(path));
        }
    }
    Ok(paths)
}

impl SystemBackend for WindowsSystem {
    fn os_name(&self) -> &'static str {
        static NAME: OnceLock<String> = OnceLock::new();
        NAME.get_or_init(|| read_os_name().unwrap_or_else(|| "Windows".into()))
    }

    fn hostname(&self) -> String {
        std::env::var("COMPUTERNAME").unwrap_or_else(|_| "PC".into())
    }

    fn monotonic_ms(&self) -> u64 {
        unsafe { GetTickCount64() }
    }

    fn local_hms(&self) -> (u16, u16, u16) {
        let now = unsafe { GetLocalTime() };
        (now.wHour, now.wMinute, now.wSecond)
    }

    fn config_dir(&self) -> Result<PathBuf, String> {
        let base = std::env::var_os("APPDATA").ok_or("APPDATA unavailable")?;
        let dir = PathBuf::from(base).join("PC Remote");
        fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        Ok(dir)
    }

    fn downloads_dir(&self) -> Option<PathBuf> {
        let known = unsafe {
            SHGetKnownFolderPath(&FOLDERID_Downloads, KF_FLAG_DEFAULT, None)
                .ok()
                .map(|path| {
                    let value = std::ffi::OsString::from_wide(path.as_wide());
                    CoTaskMemFree(Some(path.0 as *const _));
                    PathBuf::from(value)
                })
        };
        known.filter(|path| path.is_absolute()).or_else(|| {
            std::env::var_os("USERPROFILE").map(|home| PathBuf::from(home).join("Downloads"))
        })
    }

    fn is_plain_directory(&self, meta: &fs::Metadata) -> bool {
        meta.is_dir()
            && !meta.file_type().is_symlink()
            && meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT.0 == 0
    }

    fn free_disk_bytes(&self, folder: &Path) -> Result<u64, String> {
        let mut free = 0u64;
        unsafe { GetDiskFreeSpaceExW(&HSTRING::from(folder), Some(&mut free), None, None) }
            .map_err(|e| e.message())?;
        Ok(free)
    }

    fn mark_of_the_web(&self, path: &Path) -> io::Result<()> {
        let mut stream = path.as_os_str().to_owned();
        stream.push(":Zone.Identifier");
        let mut file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(PathBuf::from(stream))?;
        file.write_all(MARK_OF_THE_WEB)?;
        file.sync_all()
    }

    fn move_file_unique(
        &self,
        from: &Path,
        folder: &Path,
        name: &SafeName,
        transfer_id: Uuid,
    ) -> Result<String, String> {
        let source = HSTRING::from(from);
        for candidate in name.candidates(transfer_id) {
            let target = HSTRING::from(folder.join(&candidate).as_path());
            match unsafe { MoveFileExW(&source, &target, MOVEFILE_WRITE_THROUGH) } {
                Ok(()) => return Ok(candidate),
                Err(error)
                    if error.code() == ERROR_ALREADY_EXISTS.to_hresult()
                        || error.code() == ERROR_FILE_EXISTS.to_hresult() => {}
                Err(error) => return Err(error.message()),
            }
        }
        Err("nenhum nome livre".into())
    }

    fn secret_file_name(&self) -> &'static str {
        "identity.dpapi"
    }

    fn protect_secret(&self, data: &[u8], decrypt: bool) -> Result<Vec<u8>, String> {
        unsafe {
            let input = CRYPT_INTEGER_BLOB {
                cbData: data.len() as u32,
                pbData: data.as_ptr() as *mut u8,
            };
            let mut output = CRYPT_INTEGER_BLOB::default();
            let result = if decrypt {
                CryptUnprotectData(
                    &input,
                    None,
                    None,
                    None,
                    None,
                    CRYPTPROTECT_UI_FORBIDDEN,
                    &mut output,
                )
            } else {
                CryptProtectData(
                    &input,
                    None,
                    None,
                    None,
                    None,
                    CRYPTPROTECT_UI_FORBIDDEN,
                    &mut output,
                )
            };
            result.map_err(|e| e.to_string())?;
            let bytes = std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec();
            let _ = LocalFree(Some(HLOCAL(output.pbData as *mut _)));
            Ok(bytes)
        }
    }

    fn save_secret_file(&self, path: &Path, bytes: &[u8]) -> Result<(), String> {
        let temp = path.with_extension("tmp");
        fs::write(&temp, bytes).map_err(|e| e.to_string())?;
        fs::rename(&temp, path).map_err(|e| e.to_string())
    }

    fn open_url(&self, url: &str) -> Result<(), ()> {
        shell_open(&HSTRING::from(url))
    }

    fn open_youtube_url(&self, url: &str) -> Result<(), ()> {
        let initialized =
            unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) }
                .is_ok();
        let opened = shell_open(&HSTRING::from(url));
        if initialized {
            unsafe { CoUninitialize() };
        }
        opened
    }

    fn open_folder(&self, folder: &Path) -> Result<(), ()> {
        shell_open(&HSTRING::from(folder))
    }

    fn pick_files(&self, owner: isize) -> Result<Vec<PathBuf>, String> {
        let initialized =
            unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) }
                .is_ok();
        let result = unsafe { show_file_dialog(owner) };
        if initialized {
            unsafe { CoUninitialize() };
        }
        result.map_err(|error| {
            format!(
                "Não foi possível abrir a escolha de arquivos: {}",
                error.message()
            )
        })
    }

    fn exit_after_update_install(&self) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_follow_the_build_number_and_edition() {
        assert_eq!(describe_windows("26100", "Professional"), "Windows 11 Pro");
        assert_eq!(describe_windows("22000", "Core"), "Windows 11 Home");
        assert_eq!(
            describe_windows("19045", "Enterprise"),
            "Windows 10 Enterprise"
        );
        assert_eq!(describe_windows("19045", "IoTEnterprise"), "Windows 10");
        assert_eq!(describe_windows("x", ""), "Windows");
    }

    #[test]
    fn this_pc_reports_a_short_name() {
        let name = WindowsSystem.os_name();
        assert!(name.starts_with("Windows") && name.len() <= 64, "{name}");
    }
}
