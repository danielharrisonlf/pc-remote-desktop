//! Opens release pages and installer downloads in the user's default browser.
//!
//! The webview cannot navigate away from the app, so update links go through
//! this command. Only this project's GitHub release URLs are accepted: the
//! frontend renders text from the GitHub API and must not be able to turn it
//! into an arbitrary shell launch.

const ALLOWED_PREFIXES: [&str; 2] = [
    "https://github.com/danielharrisonlf/pc-remote-desktop/releases",
    "https://github.com/danielharrisonlf/pc-remote-android/releases",
];
const MAX_URL_LENGTH: usize = 2048;

pub fn validate(url: &str) -> Result<(), String> {
    let well_formed = !url.is_empty()
        && url.len() <= MAX_URL_LENGTH
        && url
            .bytes()
            .all(|b| b.is_ascii_graphic() && !matches!(b, b'"' | b'\\' | b'<' | b'>' | b'`'));
    let allowed = ALLOWED_PREFIXES.iter().any(|prefix| {
        url.strip_prefix(prefix)
            .is_some_and(|rest| rest.is_empty() || rest.starts_with('/'))
    });
    if well_formed && allowed && !url.contains("/../") && !url.contains('@') {
        Ok(())
    } else {
        Err("Endereço de download não permitido.".into())
    }
}

pub fn open(url: &str) -> Result<(), String> {
    validate(url)?;
    shell_open(&windows::core::HSTRING::from(url))
        .map_err(|()| "Não foi possível abrir o navegador padrão.".into())
}

const YOUTUBE_SEARCH: &str = "https://www.youtube.com/results?search_query=";

/// application/x-www-form-urlencoded: space as `+`, unreserved bytes kept, the rest `%XX`.
fn form_encode(text: &str) -> String {
    let mut out = String::with_capacity(text.len() * 3);
    for byte in text.bytes() {
        match byte {
            b' ' => out.push('+'),
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'*' => {
                out.push(byte as char)
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

/// search.v1: the phone sends only the words; the origin is fixed here.
pub fn youtube_search_url(query: &str) -> String {
    format!("{YOUTUBE_SEARCH}{}", form_encode(query))
}

/// Opens the search in the default browser. Runs on a blocking thread, so it sets up COM for ShellExecute.
pub fn open_youtube_search(query: &str) -> Result<(), ()> {
    use windows::Win32::System::Com::{
        CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE,
    };
    let url = youtube_search_url(query);
    let initialized =
        unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) }.is_ok();
    let opened = shell_open(&windows::core::HSTRING::from(url.as_str()));
    if initialized {
        unsafe { CoUninitialize() };
    }
    opened
}

/// Opens a directory in Explorer. The caller passes the app's own received-files folder.
pub fn open_folder(folder: &std::path::Path) -> Result<(), String> {
    if !folder.is_dir() {
        return Err("Pasta não encontrada.".into());
    }
    shell_open(&windows::core::HSTRING::from(folder))
        .map_err(|()| "Não foi possível abrir a pasta no Explorador.".into())
}

fn shell_open(target: &windows::core::HSTRING) -> Result<(), ()> {
    use windows::core::{HSTRING, PCWSTR};
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
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

#[cfg(test)]
mod tests {
    use super::{validate, youtube_search_url};

    #[test]
    fn search_urls_encode_every_query_into_the_fixed_origin() {
        assert_eq!(
            youtube_search_url("vídeos de Node.js"),
            "https://www.youtube.com/results?search_query=v%C3%ADdeos+de+Node.js"
        );
        for hostile in [
            "a&search_query=b",
            "x#frag",
            "../../etc",
            "https://evil.example",
            "a?b=c",
            "\"><script>",
        ] {
            let url = youtube_search_url(hostile);
            let rest = url
                .strip_prefix("https://www.youtube.com/results?search_query=")
                .unwrap();
            assert!(
                rest.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"-_.*+%".contains(&b)),
                "{url}"
            );
        }
    }

    #[test]
    fn accepts_project_release_pages_and_assets() {
        assert!(validate("https://github.com/danielharrisonlf/pc-remote-desktop/releases").is_ok());
        assert!(validate(
            "https://github.com/danielharrisonlf/pc-remote-desktop/releases/download/v0.3.0/pc-remote-desktop-0.3.0-x64-setup.exe"
        )
        .is_ok());
        assert!(validate(
            "https://github.com/danielharrisonlf/pc-remote-desktop/releases/tag/v0.3.0"
        )
        .is_ok());
    }

    #[test]
    fn rejects_other_hosts_schemes_and_lookalikes() {
        for url in [
            "",
            "http://github.com/danielharrisonlf/pc-remote-desktop/releases",
            "https://github.com/danielharrisonlf/pc-remote-desktop/releasesx",
            "https://github.com/danielharrisonlf/pc-remote-desktop/releases/../../evil",
            "https://github.com/danielharrisonlf/pc-remote-desktop/releases@evil.example",
            "https://github.com.evil.example/danielharrisonlf/pc-remote-desktop/releases",
            "https://github.com/other/pc-remote-desktop/releases",
            "file:///C:/Windows/System32/cmd.exe",
            "https://github.com/danielharrisonlf/pc-remote-desktop/releases/ x",
            "https://github.com/danielharrisonlf/pc-remote-desktop/releases/\"x",
        ] {
            assert!(validate(url).is_err(), "{url}");
        }
    }
}
