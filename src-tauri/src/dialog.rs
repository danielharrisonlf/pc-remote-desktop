//! Native "Enviar ao celular" picker. Paths stay in Rust; the webview never reads files.
use std::path::PathBuf;
use windows::{
    core::{w, HSTRING},
    Win32::{
        Foundation::{ERROR_CANCELLED, HWND},
        System::Com::{
            CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize, CLSCTX_INPROC_SERVER,
            COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE,
        },
        UI::Shell::{
            FileOpenDialog, IFileOpenDialog, FOS_ALLOWMULTISELECT, FOS_FILEMUSTEXIST,
            FOS_FORCEFILESYSTEM, SIGDN_FILESYSPATH,
        },
    },
};

/// Shows the dialog owned by `owner` (0 for none) and returns the chosen files; empty when cancelled.
/// Call from a thread that may block: it initializes its own single-threaded apartment.
pub fn pick_files(owner: isize) -> Result<Vec<PathBuf>, String> {
    let initialized =
        unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) }.is_ok();
    let result = unsafe { show(owner) };
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

unsafe fn show(owner: isize) -> windows::core::Result<Vec<PathBuf>> {
    let dialog: IFileOpenDialog = CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER)?;
    let options = dialog.GetOptions()?;
    dialog.SetOptions(options | FOS_ALLOWMULTISELECT | FOS_FORCEFILESYSTEM | FOS_FILEMUSTEXIST)?;
    dialog.SetTitle(&HSTRING::from("Enviar arquivos ao celular"))?;
    dialog.SetOkButtonLabel(w!("Enviar"))?;
    let parent = (owner != 0).then(|| HWND(owner as *mut _));
    if let Err(error) = dialog.Show(parent) {
        return if error.code() == ERROR_CANCELLED.to_hresult() {
            Ok(Vec::new())
        } else {
            Err(error)
        };
    }
    let items = dialog.GetResults()?;
    let mut paths = Vec::new();
    for index in 0..items.GetCount()? {
        let name = items.GetItemAt(index)?.GetDisplayName(SIGDN_FILESYSPATH)?;
        let path = name.to_string();
        CoTaskMemFree(Some(name.0 as *const _));
        if let Ok(path) = path {
            paths.push(PathBuf::from(path));
        }
    }
    Ok(paths)
}
