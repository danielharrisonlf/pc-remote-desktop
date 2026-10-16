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
    use windows::core::{HSTRING, PCWSTR};
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    let operation = HSTRING::from("open");
    let target = HSTRING::from(url);
    // ShellExecute returns a value greater than 32 on success.
    let result = unsafe {
        ShellExecuteW(
            None,
            &operation,
            &target,
            PCWSTR::null(),
            PCWSTR::null(),
            SW_SHOWNORMAL,
        )
    };
    if result.0 as isize > 32 {
        Ok(())
    } else {
        Err("Não foi possível abrir o navegador padrão.".into())
    }
}

#[cfg(test)]
mod tests {
    use super::validate;

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
