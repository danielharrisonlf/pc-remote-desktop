//! The Windows name the phone shows under the PC name (info.v1).
use std::sync::OnceLock;

/// "Windows 11 Pro", "Windows 10 Home"… read once from the registry.
pub fn os_name() -> &'static str {
    static NAME: OnceLock<String> = OnceLock::new();
    NAME.get_or_init(|| read().unwrap_or_else(|| "Windows".into()))
}

fn read() -> Option<String> {
    let key = windows_registry::LOCAL_MACHINE
        .open("SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion")
        .ok()?;
    let build = key.get_string("CurrentBuildNumber").ok()?;
    let edition = key.get_string("EditionID").unwrap_or_default();
    Some(describe(&build, &edition))
}

/// ProductName still says "Windows 10" on Windows 11, so the build decides.
fn describe(build: &str, edition: &str) -> String {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_follow_the_build_number_and_edition() {
        assert_eq!(describe("26100", "Professional"), "Windows 11 Pro");
        assert_eq!(describe("22000", "Core"), "Windows 11 Home");
        assert_eq!(describe("19045", "Enterprise"), "Windows 10 Enterprise");
        assert_eq!(describe("19045", "IoTEnterprise"), "Windows 10");
        assert_eq!(describe("x", ""), "Windows");
    }

    #[test]
    fn this_pc_reports_a_short_name() {
        let name = os_name();
        assert!(name.starts_with("Windows") && name.len() <= 64, "{name}");
    }
}
