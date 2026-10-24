//! User-triggered, narrowly scoped Windows Firewall access repair.
//!
//! The generated Windows prompt-rule name is used only as a narrow recognition
//! heuristic alongside the local store, app path, display name, protocol,
//! direction, and block action. It is not proof of who created the rule.

use serde::{Deserialize, Serialize};
use std::path::Path;
#[cfg(windows)]
use std::path::PathBuf;

const TCP_RULE_NAME: &str = "PCRemote.LAN.TCP.v1";
const UDP_RULE_NAME: &str = "PCRemote.LAN.MDNS.v1";
const FIXED_RULE_NAMES: [&str; 2] = [TCP_RULE_NAME, UDP_RULE_NAME];
const REPAIR_MESSAGE: &str =
    "Permissões locais configuradas. Tente conectar novamente pelo celular.";
const RESTRICTED_MESSAGE: &str =
    "O Firewall do Windows ou uma política de rede ainda pode impedir a conexão.";
const UNKNOWN_MESSAGE: &str = "Não foi possível verificar as permissões de rede do Windows.";

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct NetworkAccessStatus {
    pub supported: bool,
    pub state: NetworkAccessState,
    pub message: String,
    pub profile: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum NetworkAccessState {
    Configured,
    Restricted,
    Unknown,
    Cancelled,
}

impl NetworkAccessStatus {
    fn windows(state: NetworkAccessState, message: &str, profile: Option<String>) -> Self {
        Self {
            supported: true,
            state,
            message: message.to_string(),
            profile,
        }
    }

    fn unsupported() -> Self {
        Self {
            supported: false,
            state: NetworkAccessState::Unknown,
            message: "Esta configuração está disponível apenas no Windows.".to_string(),
            profile: None,
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct FirewallSnapshot {
    #[serde(default)]
    active_profiles: Vec<String>,
    #[serde(default)]
    profiles: Vec<ProfileFact>,
    #[serde(default)]
    rules: Vec<RuleFact>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct ProfileFact {
    name: String,
    enabled: Option<String>,
    allow_local_firewall_rules: Option<String>,
    allow_inbound_rules: Option<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct RuleFact {
    name: String,
    display_name: Option<String>,
    store: String,
    policy_source_type: Option<String>,
    program: Option<String>,
    protocol: Option<String>,
    direction: Option<String>,
    action: Option<String>,
    enabled: Option<String>,
    profile: Option<String>,
    local_port: Option<String>,
    remote_port: Option<String>,
    local_address: Option<String>,
    remote_address: Option<String>,
    edge_traversal_policy: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct RepairPlan {
    executable: String,
    disable_prompt_blocks: Vec<String>,
    create_or_update: Vec<&'static str>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PlanError {
    ManagedPolicy,
    AmbiguousOwnedRule,
}

fn normalize_path(path: &str) -> String {
    path.replace('/', "\\")
        .trim_end_matches('\\')
        .to_lowercase()
}

fn value_is_true(value: Option<&str>) -> bool {
    matches!(value, Some(v) if v.eq_ignore_ascii_case("true") || v.eq_ignore_ascii_case("enabled"))
}

fn profile_disallows_local_rules(snapshot: &FirewallSnapshot) -> bool {
    snapshot.profiles.iter().any(|p| {
        matches!(p.name.as_str(), "Private" | "Public")
            && (p.allow_local_firewall_rules.as_deref().is_some_and(|v| {
                v.eq_ignore_ascii_case("false") || v.eq_ignore_ascii_case("disabled")
            }) || p.allow_inbound_rules.as_deref().is_some_and(|v| {
                v.eq_ignore_ascii_case("false") || v.eq_ignore_ascii_case("disabled")
            }))
    })
}

fn parse_generated_query_user_name(name: &str, executable: &str) -> Option<&'static str> {
    let (protocol, rest) = if let Some(rest) = name.strip_prefix("TCP Query User{") {
        ("TCP", rest)
    } else if let Some(rest) = name.strip_prefix("UDP Query User{") {
        ("UDP", rest)
    } else {
        return None;
    };
    let (guid, suffix) = rest.split_once('}')?;
    if guid.len() != 36
        || !guid.chars().enumerate().all(|(i, c)| match i {
            8 | 13 | 18 | 23 => c == '-',
            _ => c.is_ascii_hexdigit(),
        })
        || normalize_path(suffix) != normalize_path(executable)
    {
        return None;
    }
    Some(if protocol == "TCP" { "TCP" } else { "UDP" })
}

fn is_recognized_local_prompt_block(rule: &RuleFact, executable: &str) -> bool {
    let Some(expected_protocol) = parse_generated_query_user_name(&rule.name, executable) else {
        return false;
    };
    let expected_display = Path::new(executable)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or_default();
    rule.store.eq_ignore_ascii_case("PersistentStore")
        && prompt_block_matches_identity(rule, executable, expected_protocol)
        && rule.display_name.as_deref().is_some_and(|display| {
            display.eq_ignore_ascii_case("PC Remote")
                || display.eq_ignore_ascii_case(expected_display)
        })
        && is_local_policy_source(rule.policy_source_type.as_deref())
}

fn prompt_block_matches_identity(rule: &RuleFact, executable: &str, protocol: &str) -> bool {
    rule.program
        .as_deref()
        .is_some_and(|program| normalize_path(program) == normalize_path(executable))
        && rule.protocol.as_deref().is_some_and(|v| {
            v.eq_ignore_ascii_case(protocol)
                || (protocol == "TCP" && v == "6")
                || (protocol == "UDP" && v == "17")
        })
        && rule
            .direction
            .as_deref()
            .is_some_and(|v| v.eq_ignore_ascii_case("Inbound"))
        && rule
            .action
            .as_deref()
            .is_some_and(|v| v.eq_ignore_ascii_case("Block"))
        && value_is_true(rule.enabled.as_deref())
}

fn is_local_policy_source(source: Option<&str>) -> bool {
    source.map_or(true, |value| {
        value.is_empty()
            || value.eq_ignore_ascii_case("Local")
            || value.eq_ignore_ascii_case("PersistentStore")
    })
}

fn is_effective_policy_prompt_block(rule: &RuleFact, executable: &str) -> bool {
    parse_generated_query_user_name(&rule.name, executable)
        .is_some_and(|protocol| prompt_block_matches_identity(rule, executable, protocol))
        && rule.store.eq_ignore_ascii_case("ActiveStore")
        && rule
            .policy_source_type
            .as_deref()
            .is_some_and(|v| !is_local_policy_source(Some(v)))
}

fn is_active_block_for_application(rule: &RuleFact, executable: &str) -> bool {
    rule.store.eq_ignore_ascii_case("ActiveStore")
        && rule.program.as_deref().map_or(true, |program| {
            program.eq_ignore_ascii_case("Any")
                || normalize_path(program) == normalize_path(executable)
        })
        && rule
            .direction
            .as_deref()
            .is_some_and(|v| v.eq_ignore_ascii_case("Inbound"))
        && rule
            .action
            .as_deref()
            .is_some_and(|v| v.eq_ignore_ascii_case("Block"))
        && value_is_true(rule.enabled.as_deref())
        && protocol_and_port_may_block_target(rule)
}

fn protocol_and_port_may_block_target(rule: &RuleFact) -> bool {
    let ports = rule.local_port.as_deref().map_or([true, true], |value| {
        let mut hits = [false, false];
        for item in value.split(',').map(str::trim) {
            if item.eq_ignore_ascii_case("Any") || item == "*" {
                return [true, true];
            }
            let mut range = item.splitn(2, '-');
            let start = range.next().and_then(|n| n.parse::<u16>().ok());
            let end = range.next().and_then(|n| n.parse::<u16>().ok()).or(start);
            let (Some(start), Some(end)) = (start, end) else {
                return [true, true];
            };
            hits[0] |= start <= 43127 && 43127 <= end;
            hits[1] |= start <= 5353 && 5353 <= end;
        }
        hits
    });
    match rule.protocol.as_deref() {
        Some(protocol) if protocol.eq_ignore_ascii_case("TCP") || protocol == "6" => ports[0],
        Some(protocol) if protocol.eq_ignore_ascii_case("UDP") || protocol == "17" => ports[1],
        Some(protocol) if protocol.eq_ignore_ascii_case("Any") || protocol == "256" => {
            ports[0] || ports[1]
        }
        Some(_) => false,
        None => true,
    }
}

fn is_ambiguous_local_prompt_block(rule: &RuleFact, executable: &str) -> bool {
    let Some(protocol) = parse_generated_query_user_name(&rule.name, executable) else {
        return false;
    };
    rule.store.eq_ignore_ascii_case("PersistentStore")
        && prompt_block_matches_identity(rule, executable, protocol)
        && !is_recognized_local_prompt_block(rule, executable)
}

fn target_rule_is_safe_to_manage(rule: &RuleFact, executable: &str) -> bool {
    rule.store.eq_ignore_ascii_case("PersistentStore")
        && rule
            .program
            .as_deref()
            .is_some_and(|program| normalize_path(program) == normalize_path(executable))
        && rule.protocol.is_some()
        && rule.local_port.is_some()
        && rule.remote_port.is_some()
        && rule.local_address.is_some()
        && rule.remote_address.is_some()
}

fn plan_repair(snapshot: &FirewallSnapshot, executable: &str) -> Result<RepairPlan, PlanError> {
    if profile_disallows_local_rules(snapshot) {
        return Err(PlanError::ManagedPolicy);
    }

    for fixed_name in FIXED_RULE_NAMES {
        for rule in snapshot.rules.iter().filter(|rule| rule.name == fixed_name) {
            if rule.store.eq_ignore_ascii_case("PersistentStore")
                && !target_rule_is_safe_to_manage(rule, executable)
            {
                return Err(PlanError::AmbiguousOwnedRule);
            }
            if rule.store.eq_ignore_ascii_case("ActiveStore")
                && rule
                    .policy_source_type
                    .as_deref()
                    .is_some_and(|source| !is_local_policy_source(Some(source)))
            {
                return Err(PlanError::ManagedPolicy);
            }
        }
    }

    if snapshot
        .rules
        .iter()
        .any(|rule| is_effective_policy_prompt_block(rule, executable))
    {
        return Err(PlanError::ManagedPolicy);
    }

    if snapshot
        .rules
        .iter()
        .any(|rule| is_ambiguous_local_prompt_block(rule, executable))
    {
        return Err(PlanError::AmbiguousOwnedRule);
    }

    for blocker in snapshot
        .rules
        .iter()
        .filter(|rule| is_active_block_for_application(rule, executable))
    {
        let has_recognized_local_source = snapshot.rules.iter().any(|local| {
            local.store.eq_ignore_ascii_case("PersistentStore")
                && local.name == blocker.name
                && is_recognized_local_prompt_block(local, executable)
        });
        if !has_recognized_local_source {
            return Err(
                if blocker
                    .policy_source_type
                    .as_deref()
                    .is_some_and(|source| !is_local_policy_source(Some(source)))
                {
                    PlanError::ManagedPolicy
                } else {
                    PlanError::AmbiguousOwnedRule
                },
            );
        }
    }

    let disable_prompt_blocks = snapshot
        .rules
        .iter()
        .filter(|rule| is_recognized_local_prompt_block(rule, executable))
        .map(|rule| rule.name.clone())
        .collect();

    Ok(RepairPlan {
        executable: executable.to_string(),
        disable_prompt_blocks,
        create_or_update: FIXED_RULE_NAMES.into_iter().collect(),
    })
}

fn is_target_rule_effective(rule: &RuleFact, expected: &str, executable: &str) -> bool {
    let expected_protocol = if expected == TCP_RULE_NAME {
        "TCP"
    } else {
        "UDP"
    };
    let expected_port = if expected == TCP_RULE_NAME {
        "43127"
    } else {
        "5353"
    };
    rule.store.eq_ignore_ascii_case("ActiveStore")
        && rule.name == expected
        && rule
            .program
            .as_deref()
            .is_some_and(|program| normalize_path(program) == normalize_path(executable))
        && rule
            .direction
            .as_deref()
            .is_some_and(|v| v.eq_ignore_ascii_case("Inbound"))
        && rule
            .action
            .as_deref()
            .is_some_and(|v| v.eq_ignore_ascii_case("Allow"))
        && value_is_true(rule.enabled.as_deref())
        && rule.protocol.as_deref().is_some_and(|v| {
            v.eq_ignore_ascii_case(expected_protocol)
                || (expected_protocol == "TCP" && v == "6")
                || (expected_protocol == "UDP" && v == "17")
        })
        && rule
            .local_port
            .as_deref()
            .is_some_and(|v| v == expected_port)
        && rule
            .remote_port
            .as_deref()
            .is_some_and(|v| v.eq_ignore_ascii_case("Any"))
        && rule
            .remote_address
            .as_deref()
            .is_some_and(|v| v.eq_ignore_ascii_case("LocalSubnet"))
        && rule
            .local_address
            .as_deref()
            .is_some_and(|v| v.eq_ignore_ascii_case("Any"))
        && rule.profile.as_deref().is_some_and(|v| {
            let mut profiles = v
                .split(',')
                .map(|p| p.trim().to_ascii_lowercase())
                .collect::<Vec<_>>();
            profiles.sort();
            profiles == ["private", "public"]
        })
        && rule
            .edge_traversal_policy
            .as_deref()
            .is_some_and(|v| v.eq_ignore_ascii_case("Block"))
}

fn configured(snapshot: &FirewallSnapshot, executable: &str) -> bool {
    if profile_disallows_local_rules(snapshot)
        || snapshot.active_profiles.is_empty()
        || snapshot.active_profiles.iter().any(|profile| {
            !profile.eq_ignore_ascii_case("Private") && !profile.eq_ignore_ascii_case("Public")
        })
    {
        return false;
    }
    let has_tcp = snapshot
        .rules
        .iter()
        .any(|rule| is_target_rule_effective(rule, TCP_RULE_NAME, executable));
    let has_udp = snapshot
        .rules
        .iter()
        .any(|rule| is_target_rule_effective(rule, UDP_RULE_NAME, executable));
    let has_enabled_prompt_block = snapshot.rules.iter().any(|rule| {
        rule.enabled
            .as_deref()
            .is_some_and(|v| value_is_true(Some(v)))
            && is_recognized_local_prompt_block(rule, executable)
    });
    let has_other_effective_block = snapshot
        .rules
        .iter()
        .any(|rule| is_active_block_for_application(rule, executable));
    has_tcp && has_udp && !has_enabled_prompt_block && !has_other_effective_block
}

fn profile_label(snapshot: &FirewallSnapshot) -> Option<String> {
    if snapshot.active_profiles.is_empty() {
        return None;
    }
    let mut profiles = snapshot.active_profiles.clone();
    profiles.sort_unstable_by_key(|name| name.to_ascii_lowercase());
    profiles.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
    Some(profiles.join(", "))
}

#[tauri::command]
pub async fn network_access_status() -> Result<NetworkAccessStatus, String> {
    #[cfg(windows)]
    {
        return tauri::async_runtime::spawn_blocking(status_windows)
            .await
            .map_err(|_| "Não foi possível verificar o Firewall do Windows.".to_string());
    }
    #[cfg(not(windows))]
    {
        Ok(NetworkAccessStatus::unsupported())
    }
}

static REPAIR_IN_FLIGHT: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

struct RepairFlight;

impl RepairFlight {
    fn try_start() -> Option<Self> {
        REPAIR_IN_FLIGHT
            .compare_exchange(
                false,
                true,
                std::sync::atomic::Ordering::Acquire,
                std::sync::atomic::Ordering::Relaxed,
            )
            .ok()
            .map(|_| Self)
    }
}

impl Drop for RepairFlight {
    fn drop(&mut self) {
        REPAIR_IN_FLIGHT.store(false, std::sync::atomic::Ordering::Release);
    }
}

#[tauri::command]
pub async fn repair_network_access() -> Result<NetworkAccessStatus, String> {
    #[cfg(windows)]
    {
        return tauri::async_runtime::spawn_blocking(repair_windows)
            .await
            .map_err(|_| "Não foi possível configurar o Firewall do Windows.".to_string());
    }
    #[cfg(not(windows))]
    {
        Ok(NetworkAccessStatus::unsupported())
    }
}

#[cfg(windows)]
fn status_windows() -> NetworkAccessStatus {
    match windows_snapshot() {
        Ok((snapshot, _)) => {
            let profile = profile_label(&snapshot);
            let (state, message) = if configured(&snapshot, &current_executable_string()) {
                (
                    NetworkAccessState::Configured,
                    "As permissões locais de rede estão configuradas.",
                )
            } else {
                (NetworkAccessState::Restricted, RESTRICTED_MESSAGE)
            };
            NetworkAccessStatus::windows(state, message, profile)
        }
        Err(()) => NetworkAccessStatus::windows(NetworkAccessState::Unknown, UNKNOWN_MESSAGE, None),
    }
}

#[cfg(windows)]
fn repair_windows() -> NetworkAccessStatus {
    let Some(_flight) = RepairFlight::try_start() else {
        return NetworkAccessStatus::windows(
            NetworkAccessState::Restricted,
            "Uma configuração de rede já está em andamento.",
            None,
        );
    };

    let executable = match std::env::current_exe() {
        Ok(path) => path,
        Err(_) => {
            return NetworkAccessStatus::windows(NetworkAccessState::Unknown, UNKNOWN_MESSAGE, None)
        }
    };
    let executable = executable.to_string_lossy().to_string();
    let (before, _) = match windows_snapshot_for(&executable) {
        Ok(value) => value,
        Err(()) => {
            return NetworkAccessStatus::windows(NetworkAccessState::Unknown, UNKNOWN_MESSAGE, None)
        }
    };
    let profile = profile_label(&before);
    if configured(&before, &executable) {
        return NetworkAccessStatus::windows(
            NetworkAccessState::Configured,
            "As permissões locais de rede já estão configuradas.",
            profile,
        );
    }
    struct WindowsExecutor;
    impl PlanExecutor for WindowsExecutor {
        fn apply(&mut self, plan: &RepairPlan) -> Result<ExecutionResult, ()> {
            execute_windows_plan(plan)
        }
    }

    match run_repair_plan(&before, &executable, &mut WindowsExecutor) {
        ExecutionResult::Configured(snapshot) if configured(&snapshot, &executable) => {
            NetworkAccessStatus::windows(
                NetworkAccessState::Configured,
                REPAIR_MESSAGE,
                profile_label(&snapshot),
            )
        }
        ExecutionResult::Cancelled(snapshot) => NetworkAccessStatus::windows(
            NetworkAccessState::Cancelled,
            "A solicitação de administrador foi cancelada. Nenhuma permissão foi alterada.",
            snapshot.and_then(|facts| profile_label(&facts)).or(profile),
        ),
        ExecutionResult::Restricted(snapshot) => NetworkAccessStatus::windows(
            NetworkAccessState::Restricted,
            RESTRICTED_MESSAGE,
            snapshot.and_then(|facts| profile_label(&facts)).or(profile),
        ),
        ExecutionResult::Configured(_) => {
            NetworkAccessStatus::windows(NetworkAccessState::Unknown, UNKNOWN_MESSAGE, profile)
        }
    }
}

#[cfg(windows)]
fn current_executable_string() -> String {
    std::env::current_exe()
        .map(|path| path.to_string_lossy().to_string())
        .unwrap_or_default()
}

#[cfg(windows)]
fn windows_snapshot() -> Result<(FirewallSnapshot, String), ()> {
    let executable = current_executable_string();
    windows_snapshot_for(&executable)
}

#[cfg(windows)]
fn windows_snapshot_for(executable: &str) -> Result<(FirewallSnapshot, String), ()> {
    let script = snapshot_script(executable);
    let output = run_powershell(&script)?;
    let snapshot = serde_json::from_str::<FirewallSnapshot>(&output).map_err(|_| ())?;
    Ok((snapshot, output))
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum ExecutionResult {
    Configured(FirewallSnapshot),
    Restricted(Option<FirewallSnapshot>),
    Cancelled(Option<FirewallSnapshot>),
}

trait PlanExecutor {
    fn apply(&mut self, plan: &RepairPlan) -> Result<ExecutionResult, ()>;
}

fn run_repair_plan(
    before: &FirewallSnapshot,
    executable: &str,
    executor: &mut impl PlanExecutor,
) -> ExecutionResult {
    let plan = match plan_repair(before, executable) {
        Ok(plan) => plan,
        Err(_) => return ExecutionResult::Restricted(Some(before.clone())),
    };
    match executor.apply(&plan) {
        Ok(ExecutionResult::Configured(snapshot)) if configured(&snapshot, executable) => {
            ExecutionResult::Configured(snapshot)
        }
        Ok(ExecutionResult::Configured(snapshot))
        | Ok(ExecutionResult::Restricted(Some(snapshot))) => {
            ExecutionResult::Restricted(Some(snapshot))
        }
        Ok(other) => other,
        Err(()) => ExecutionResult::Restricted(Some(before.clone())),
    }
}

/// Installer-only entry point. It shares the same repair implementation as the UI
/// and returns stable process exit codes for the NSIS post-install hook.
pub fn configure_network_for_installer() -> i32 {
    #[cfg(windows)]
    {
        return match repair_windows().state {
            NetworkAccessState::Configured => 0,
            NetworkAccessState::Cancelled => 2,
            NetworkAccessState::Restricted | NetworkAccessState::Unknown => 3,
        };
    }
    #[cfg(not(windows))]
    {
        3
    }
}

#[cfg(windows)]
fn execute_windows_plan(plan: &RepairPlan) -> Result<ExecutionResult, ()> {
    let mut disable_names = plan.disable_prompt_blocks.clone();
    disable_names.sort();
    disable_names.dedup();
    let child_encoded = encode_powershell_utf16(&apply_script(&plan.executable, &disable_names));
    // Leave room for the executable, switches and ShellExecute quoting on Windows.
    if child_encoded.len() > 30_000 {
        return Err(());
    }
    let powershell = ps_literal(&powershell_path()?.to_string_lossy());
    let script = format!(
        r#"
$ErrorActionPreference = 'Stop'
$encodedChild = '{child_encoded}'
$powershell = {powershell}
function Emit($state, $message) {{
  $result = [pscustomobject]@{{ supported=$true; state=$state; message=$message; profile=$null }}
  [Console]::OutputEncoding = New-Object System.Text.UTF8Encoding($false)
  [Console]::Out.WriteLine((ConvertTo-Json -InputObject $result -Compress -Depth 5))
}}
function Test-IsAdmin {{
  $identity = [Security.Principal.WindowsIdentity]::GetCurrent()
  $principal = New-Object Security.Principal.WindowsPrincipal($identity)
  return $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
}}
try {{
  $arguments = "-NoProfile -NonInteractive -EncodedCommand $encodedChild"
  if (Test-IsAdmin) {{
    $child = Start-Process -FilePath $powershell -ArgumentList $arguments -WindowStyle Hidden -PassThru -Wait
  }} else {{
    try {{
      $child = Start-Process -FilePath $powershell -ArgumentList $arguments -Verb RunAs -WindowStyle Hidden -PassThru -Wait
    }} catch {{
      $cancelled = $false
      $failure = $_.Exception
      while ($null -ne $failure) {{
        if ($failure.NativeErrorCode -eq 1223 -or (([int]$failure.HResult) -band 0xffff) -eq 1223) {{ $cancelled = $true }}
        $failure = $failure.InnerException
      }}
      if ($cancelled) {{
        Emit 'cancelled' 'A solicitação de administrador foi cancelada. Nenhuma permissão foi alterada.'
        exit 0
      }}
      Emit 'unknown' '{unknown}'
      exit 0
    }}
  }}
  if ($child.ExitCode -eq 0) {{ Emit 'configured' '{configured}'; exit 0 }}
  if ($child.ExitCode -eq 2) {{ Emit 'restricted' '{restricted}'; exit 0 }}
  Emit 'unknown' '{unknown}'
  exit 0
}} catch {{
  Emit 'unknown' '{unknown}'
  exit 0
}}
"#,
        child_encoded = child_encoded,
        powershell = powershell,
        configured = REPAIR_MESSAGE,
        restricted = RESTRICTED_MESSAGE,
        unknown = UNKNOWN_MESSAGE,
    );
    let output = run_powershell(&script)?;
    let status = serde_json::from_str::<NetworkAccessStatus>(&output).map_err(|_| ())?;
    match status.state {
        NetworkAccessState::Configured => {
            let (snapshot, _) = windows_snapshot_for(&plan.executable)?;
            Ok(ExecutionResult::Configured(snapshot))
        }
        NetworkAccessState::Cancelled => Ok(ExecutionResult::Cancelled(None)),
        NetworkAccessState::Restricted => {
            let snapshot = windows_snapshot_for(&plan.executable)
                .ok()
                .map(|(snapshot, _)| snapshot);
            Ok(ExecutionResult::Restricted(snapshot))
        }
        NetworkAccessState::Unknown => Err(()),
    }
}
#[cfg(windows)]
fn apply_script(executable: &str, authorized_names: &[String]) -> String {
    let mut script = apply_functions_script(executable, authorized_names);
    script.push_str(
        r#"
try {
  if (-not (Test-IsAdmin)) { exit 4 }
  if (-not (Apply-Repair)) { exit 2 }
  exit 0
} catch { exit 3 }
"#,
    );
    script
}

#[cfg(windows)]
fn apply_functions_script(executable: &str, authorized_names: &[String]) -> String {
    let executable = ps_literal(executable);
    let names = authorized_names
        .iter()
        .map(|s| ps_literal(s))
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        r#"
$ErrorActionPreference = 'Stop'
$exe = {executable}
$authorizedPromptNames = @({names})
function Read-Snapshot {{
  $activeProfiles = @()
  try {{ $activeProfiles = @((Get-NetConnectionProfile -ErrorAction Stop | ForEach-Object {{ [string]$_.NetworkCategory }} | Sort-Object -Unique)) }} catch {{ $activeProfiles = @() }}
  $profiles = @((Get-NetFirewallProfile -PolicyStore ActiveStore -ErrorAction Stop | ForEach-Object {{ [pscustomobject]@{{ name=[string]$_.Name; enabled=[string]$_.Enabled; allowLocalFirewallRules=[string]$_.AllowLocalFirewallRules; allowInboundRules=[string]$_.AllowInboundRules }} }}))
  $rules = @()
  foreach ($store in @('PersistentStore','ActiveStore')) {{
    $found = @(Get-NetFirewallRule -PolicyStore $store -ErrorAction Stop | Where-Object {{ $_.Name -in @('{tcp_name}','{udp_name}') -or $_.Name -match '^(TCP|UDP) Query User\{{[0-9A-Fa-f-]{{36}}\}}' -or ($store -eq 'ActiveStore' -and $_.Direction -eq 'Inbound' -and $_.Action -eq 'Block') }})
    foreach ($rule in $found) {{
      $apps = @(Get-NetFirewallApplicationFilter -AssociatedNetFirewallRule $rule -ErrorAction SilentlyContinue)
      $ports = @(Get-NetFirewallPortFilter -AssociatedNetFirewallRule $rule -ErrorAction SilentlyContinue)
      $addresses = @(Get-NetFirewallAddressFilter -AssociatedNetFirewallRule $rule -ErrorAction SilentlyContinue)
      $rules += [pscustomobject]@{{ name=[string]$rule.Name; displayName=[string]$rule.DisplayName; store=$store; policySourceType=[string]$rule.PolicyStoreSourceType; program=if ($apps.Count -eq 1) {{ [string]$apps[0].Program }} else {{ $null }}; protocol=if ($ports.Count -eq 1) {{ [string]$ports[0].Protocol }} else {{ $null }}; direction=[string]$rule.Direction; action=[string]$rule.Action; enabled=[string]$rule.Enabled; profile=(@($rule.Profile) -join ', '); localPort=if ($ports.Count -eq 1) {{ (@($ports[0].LocalPort) -join ', ') }} else {{ $null }}; remotePort=if ($ports.Count -eq 1) {{ (@($ports[0].RemotePort) -join ', ') }} else {{ $null }}; localAddress=if ($addresses.Count -eq 1) {{ (@($addresses[0].LocalAddress) -join ', ') }} else {{ $null }}; remoteAddress=if ($addresses.Count -eq 1) {{ (@($addresses[0].RemoteAddress) -join ', ') }} else {{ $null }}; edgeTraversalPolicy=[string]$rule.EdgeTraversalPolicy }}
    }}
  }}
  return [pscustomobject]@{{ activeProfiles=$activeProfiles; profiles=$profiles; rules=$rules }}
}}
function Apply-Repair {{
  $snapshot = Read-Snapshot
  foreach ($profile in @($snapshot.profiles | Where-Object {{ $_.name -in @('Private','Public') }})) {{ if ($profile.allowLocalFirewallRules -match '^(False|Disabled)$' -or $profile.allowInboundRules -match '^(False|Disabled)$') {{ return $false }} }}
  foreach ($name in @('{tcp_name}','{udp_name}')) {{
    $local = @($snapshot.rules | Where-Object {{ $_.store -eq 'PersistentStore' -and $_.name -eq $name }})
    if ($local.Count -gt 1) {{ return $false }}
    foreach ($rule in $local) {{ if ([string]::IsNullOrWhiteSpace($rule.program) -or [string]::IsNullOrWhiteSpace($rule.protocol) -or [string]::IsNullOrWhiteSpace($rule.localPort) -or [string]::IsNullOrWhiteSpace($rule.remotePort) -or [string]::IsNullOrWhiteSpace($rule.localAddress) -or [string]::IsNullOrWhiteSpace($rule.remoteAddress) -or -not [string]::Equals($rule.program,$exe,[StringComparison]::OrdinalIgnoreCase)) {{ return $false }} }}
    $active = @($snapshot.rules | Where-Object {{ $_.store -eq 'ActiveStore' -and $_.name -eq $name }})
    if ($active.Count -gt 1) {{ return $false }}
    foreach ($rule in $active) {{ if (($rule.policySourceType -and $rule.policySourceType -notin @('Local','PersistentStore')) -or [string]::IsNullOrWhiteSpace($rule.program) -or -not [string]::Equals($rule.program,$exe,[StringComparison]::OrdinalIgnoreCase)) {{ return $false }} }}
  }}
  foreach ($rule in @($snapshot.rules | Where-Object {{ $_.store -eq 'ActiveStore' -and $_.policySourceType -and $_.policySourceType -notin @('Local','PersistentStore') -and $_.Name -match '^(TCP|UDP) Query User\{{[0-9A-Fa-f-]{{36}}\}}' -and [string]::Equals($_.program,$exe,[StringComparison]::OrdinalIgnoreCase) -and $_.direction -eq 'Inbound' -and $_.action -eq 'Block' }})) {{ return $false }}
  $promptToDisable = @()
  foreach ($name in $authorizedPromptNames) {{
    $rules = @(Get-NetFirewallRule -PolicyStore PersistentStore -ErrorAction Stop | Where-Object {{ $_.Name -ceq $name }})
    if ($rules.Count -ne 1) {{ return $false }}
    $rule = $rules[0]
    $app = @(Get-NetFirewallApplicationFilter -AssociatedNetFirewallRule $rule -ErrorAction SilentlyContinue)
    $port = @(Get-NetFirewallPortFilter -AssociatedNetFirewallRule $rule -ErrorAction SilentlyContinue)
    $pattern = '^(TCP|UDP) Query User\{{[0-9A-Fa-f]{{8}}-[0-9A-Fa-f]{{4}}-[0-9A-Fa-f]{{4}}-[0-9A-Fa-f]{{4}}-[0-9A-Fa-f]{{12}}\}}' + [regex]::Escape($exe) + '$'
    $nameMatch = [regex]::Match($name, $pattern, [Text.RegularExpressions.RegexOptions]::IgnoreCase)
    $isExpectedName = $nameMatch.Success
    $protocol = $nameMatch.Groups[1].Value
    $expectedDisplay = [IO.Path]::GetFileName($exe)
    if (-not $isExpectedName -or $app.Count -ne 1 -or $port.Count -ne 1 -or -not [string]::Equals($app[0].Program,$exe,[StringComparison]::OrdinalIgnoreCase) -or -not ($rule.DisplayName -ieq 'PC Remote' -or $rule.DisplayName -ieq $expectedDisplay) -or $rule.Direction -ne 'Inbound' -or $rule.Action -ne 'Block' -or -not (($protocol -eq 'TCP' -and ($port[0].Protocol -eq 'TCP' -or $port[0].Protocol -eq 6)) -or ($protocol -eq 'UDP' -and ($port[0].Protocol -eq 'UDP' -or $port[0].Protocol -eq 17))) -or ($rule.PolicyStoreSourceType -and $rule.PolicyStoreSourceType -notin @('Local','PersistentStore'))) {{ return $false }}
    $promptToDisable += $rule
  }}
  $tcp = Get-NetFirewallRule -PolicyStore PersistentStore -Name '{tcp_name}' -ErrorAction SilentlyContinue
  if ($null -eq $tcp) {{ New-NetFirewallRule -PolicyStore PersistentStore -Name '{tcp_name}' -DisplayName 'PC Remote (TCP LAN)' -Program $exe -Direction Inbound -Action Allow -Protocol TCP -LocalPort 43127 -RemoteAddress LocalSubnet -Profile Private,Public -EdgeTraversalPolicy Block -Enabled True -ErrorAction Stop | Out-Null }} else {{
    Set-NetFirewallRule -InputObject $tcp -Enabled False -ErrorAction Stop | Out-Null
    Get-NetFirewallApplicationFilter -AssociatedNetFirewallRule $tcp -ErrorAction Stop | Set-NetFirewallApplicationFilter -Program $exe -ErrorAction Stop | Out-Null
    Get-NetFirewallPortFilter -AssociatedNetFirewallRule $tcp -ErrorAction Stop | Set-NetFirewallPortFilter -Protocol TCP -LocalPort 43127 -RemotePort Any -ErrorAction Stop | Out-Null
    Get-NetFirewallAddressFilter -AssociatedNetFirewallRule $tcp -ErrorAction Stop | Set-NetFirewallAddressFilter -LocalAddress Any -RemoteAddress LocalSubnet -ErrorAction Stop | Out-Null
    Set-NetFirewallRule -InputObject $tcp -Direction Inbound -Action Allow -Profile Private,Public -EdgeTraversalPolicy Block -Enabled True -ErrorAction Stop | Out-Null
  }}
  $udp = Get-NetFirewallRule -PolicyStore PersistentStore -Name '{udp_name}' -ErrorAction SilentlyContinue
  if ($null -eq $udp) {{ New-NetFirewallRule -PolicyStore PersistentStore -Name '{udp_name}' -DisplayName 'PC Remote (mDNS LAN)' -Program $exe -Direction Inbound -Action Allow -Protocol UDP -LocalPort 5353 -RemoteAddress LocalSubnet -Profile Private,Public -EdgeTraversalPolicy Block -Enabled True -ErrorAction Stop | Out-Null }} else {{
    Set-NetFirewallRule -InputObject $udp -Enabled False -ErrorAction Stop | Out-Null
    Get-NetFirewallApplicationFilter -AssociatedNetFirewallRule $udp -ErrorAction Stop | Set-NetFirewallApplicationFilter -Program $exe -ErrorAction Stop | Out-Null
    Get-NetFirewallPortFilter -AssociatedNetFirewallRule $udp -ErrorAction Stop | Set-NetFirewallPortFilter -Protocol UDP -LocalPort 5353 -RemotePort Any -ErrorAction Stop | Out-Null
    Get-NetFirewallAddressFilter -AssociatedNetFirewallRule $udp -ErrorAction Stop | Set-NetFirewallAddressFilter -LocalAddress Any -RemoteAddress LocalSubnet -ErrorAction Stop | Out-Null
    Set-NetFirewallRule -InputObject $udp -Direction Inbound -Action Allow -Profile Private,Public -EdgeTraversalPolicy Block -Enabled True -ErrorAction Stop | Out-Null
  }}
  foreach ($rule in $promptToDisable) {{ Disable-NetFirewallRule -InputObject $rule -ErrorAction Stop | Out-Null }}
  return $true
}}
function Test-IsAdmin {{
  $identity = [Security.Principal.WindowsIdentity]::GetCurrent()
  $principal = New-Object Security.Principal.WindowsPrincipal($identity)
  return $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
}}
"#,
        executable = executable,
        names = names,
        tcp_name = TCP_RULE_NAME,
        udp_name = UDP_RULE_NAME,
    )
}

#[cfg(windows)]
fn snapshot_script(executable: &str) -> String {
    format!(
        r#"
$ErrorActionPreference = 'Stop'
$exe = {executable}
$names = @('{tcp_name}', '{udp_name}')
$activeProfiles = @()
try {{ $activeProfiles = @((Get-NetConnectionProfile -ErrorAction Stop | ForEach-Object {{ [string]$_.NetworkCategory }} | Sort-Object -Unique)) }} catch {{ $activeProfiles = @() }}
$profiles = @((Get-NetFirewallProfile -PolicyStore ActiveStore -ErrorAction Stop | ForEach-Object {{ [pscustomobject]@{{ name=[string]$_.Name; enabled=[string]$_.Enabled; allowLocalFirewallRules=[string]$_.AllowLocalFirewallRules; allowInboundRules=[string]$_.AllowInboundRules }} }}))
$rules = @()
foreach ($store in @('PersistentStore','ActiveStore')) {{
  $found = @(Get-NetFirewallRule -PolicyStore $store -ErrorAction Stop | Where-Object {{ $_.Name -in $names -or $_.Name -match '^(TCP|UDP) Query User\{{[0-9A-Fa-f-]{{36}}\}}' -or ($store -eq 'ActiveStore' -and $_.Direction -eq 'Inbound' -and $_.Action -eq 'Block') }})
  foreach ($rule in $found) {{
    $apps = @(Get-NetFirewallApplicationFilter -AssociatedNetFirewallRule $rule -ErrorAction SilentlyContinue)
    $ports = @(Get-NetFirewallPortFilter -AssociatedNetFirewallRule $rule -ErrorAction SilentlyContinue)
    $addresses = @(Get-NetFirewallAddressFilter -AssociatedNetFirewallRule $rule -ErrorAction SilentlyContinue)
    $rules += [pscustomobject]@{{
      name=[string]$rule.Name; displayName=[string]$rule.DisplayName; store=$store
      policySourceType=[string]$rule.PolicyStoreSourceType
      program=if ($apps.Count -eq 1) {{ [string]$apps[0].Program }} else {{ $null }}
      protocol=if ($ports.Count -eq 1) {{ [string]$ports[0].Protocol }} else {{ $null }}
      direction=[string]$rule.Direction; action=[string]$rule.Action; enabled=[string]$rule.Enabled
      profile=(@($rule.Profile) -join ', ')
      localPort=if ($ports.Count -eq 1) {{ (@($ports[0].LocalPort) -join ', ') }} else {{ $null }}
      remotePort=if ($ports.Count -eq 1) {{ (@($ports[0].RemotePort) -join ', ') }} else {{ $null }}
      localAddress=if ($addresses.Count -eq 1) {{ (@($addresses[0].LocalAddress) -join ', ') }} else {{ $null }}
      remoteAddress=if ($addresses.Count -eq 1) {{ (@($addresses[0].RemoteAddress) -join ', ') }} else {{ $null }}
      edgeTraversalPolicy=[string]$rule.EdgeTraversalPolicy
    }}
  }}
}}
$result = [pscustomobject]@{{ activeProfiles=$activeProfiles; profiles=$profiles; rules=$rules }}
[Console]::OutputEncoding = New-Object System.Text.UTF8Encoding($false)
[Console]::Out.WriteLine((ConvertTo-Json -InputObject $result -Compress -Depth 8))
"#,
        executable = ps_literal(executable),
        tcp_name = TCP_RULE_NAME,
        udp_name = UDP_RULE_NAME,
    )
}

#[cfg(windows)]
fn ps_literal(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

#[cfg(windows)]
fn encode_powershell_utf16(script: &str) -> String {
    use base64::Engine;
    let bytes = script
        .encode_utf16()
        .flat_map(u16::to_le_bytes)
        .collect::<Vec<_>>();
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

#[cfg(windows)]
fn run_powershell(script: &str) -> Result<String, ()> {
    use std::{
        io::Write,
        os::windows::process::CommandExt,
        process::{Command, Stdio},
    };
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let executable = powershell_path()?;
    // Only a small bootstrap goes on the command line. The UTF-16 payload travels
    // over stdin as ASCII Base64, avoiding both command-line limits and temp files.
    let encoded = encode_powershell_utf16(
        r#"
$ErrorActionPreference = 'Stop'
[Console]::OutputEncoding = New-Object System.Text.UTF8Encoding($false)
try {
  $payload = [Console]::In.ReadToEnd()
  & ([ScriptBlock]::Create([Text.Encoding]::Unicode.GetString([Convert]::FromBase64String($payload))))
} catch { [Console]::Error.WriteLine($_.Exception.Message); exit 1 }
"#,
    );
    let mut child = Command::new(executable)
        .args(["-NoProfile", "-NonInteractive", "-EncodedCommand", &encoded])
        .creation_flags(CREATE_NO_WINDOW)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|_| ())?;
    let payload = encode_powershell_utf16(script);
    let write_result = child.stdin.take().ok_or(())?.write_all(payload.as_bytes());
    if write_result.is_err() {
        let _ = child.kill();
        let _ = child.wait();
        return Err(());
    }
    let output = child.wait_with_output().map_err(|_| ())?;
    if !output.status.success() {
        return Err(());
    }
    String::from_utf8(output.stdout).map_err(|_| ())
}

#[cfg(windows)]
fn powershell_path() -> Result<PathBuf, ()> {
    use std::os::windows::ffi::OsStringExt;
    let mut buffer = vec![0u16; 512];
    let len = unsafe {
        windows::Win32::System::SystemInformation::GetSystemDirectoryW(Some(&mut buffer)) as usize
    };
    if len == 0 || len >= buffer.len() {
        return Err(());
    }
    buffer.truncate(len);
    let mut path = PathBuf::from(std::ffi::OsString::from_wide(&buffer));
    path.push("WindowsPowerShell");
    path.push("v1.0");
    path.push("powershell.exe");
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(windows)]
    #[test]
    fn powershell_stdin_handles_large_unicode_payload_and_failure() {
        let script = format!(
            "# {}\n[Console]::Out.Write('permissões — São Paulo')",
            "large payload ".repeat(5_000)
        );
        assert!(encode_powershell_utf16(&script).len() > 32_767);
        assert_eq!(run_powershell(&script).unwrap(), "permissões — São Paulo");
        assert!(run_powershell("throw 'expected test failure'").is_err());
    }

    #[cfg(windows)]
    #[test]
    fn actual_powershell_repair_with_strict_mocked_netsecurity() {
        let executable = r"C:\Usuários\D'Ávila\PC Remote\pc-remote-desktop.exe";
        let names = vec![
            generated_name("TCP", executable),
            generated_name("UDP", executable),
        ];
        let mut script = apply_functions_script(executable, &names);
        // Only NetSecurity and network-profile APIs are replaced; recognition,
        // snapshot collection and Apply-Repair are the production functions.
        script.push_str(include_str!("network_access_test.ps1"));
        assert_eq!(
            run_powershell(&script)
                .expect("real PowerShell mocked runtime")
                .trim(),
            "passed"
        );
        assert!(encode_powershell_utf16(&apply_script(executable, &names)).len() <= 30_000);
    }

    #[test]
    fn effective_readback_rejects_domain_extra_profiles_and_remaining_blocks() {
        let rules = vec![
            effective_rule(TCP_RULE_NAME, exe()),
            effective_rule(UDP_RULE_NAME, exe()),
        ];
        assert!(configured(&snapshot(rules.clone()), exe()));
        let mut facts = snapshot(rules.clone());
        facts.active_profiles = vec!["DomainAuthenticated".into()];
        assert!(!configured(&facts, exe()));
        facts.active_profiles.clear();
        assert!(!configured(&facts, exe()));
        facts = snapshot(rules.clone());
        facts.rules[0].profile = Some("Domain, Private, Public".into());
        assert!(!configured(&facts, exe()));
        facts = snapshot(rules);
        let active = rule(
            &generated_name("TCP", exe()),
            "ActiveStore",
            Some(exe()),
            "TCP",
            "Inbound",
            "Block",
        );
        let mut local = active.clone();
        local.store = "PersistentStore".into();
        local.enabled = Some("False".into());
        facts.rules.extend([local, active]);
        assert!(!configured(&facts, exe()));
    }

    fn exe() -> &'static str {
        r"C:\Program Files\PC Remote\pc-remote-desktop.exe"
    }

    fn snapshot(rules: Vec<RuleFact>) -> FirewallSnapshot {
        FirewallSnapshot {
            active_profiles: vec!["Private".into()],
            profiles: vec![
                ProfileFact {
                    name: "Private".into(),
                    enabled: Some("True".into()),
                    allow_local_firewall_rules: Some("True".into()),
                    allow_inbound_rules: Some("True".into()),
                },
                ProfileFact {
                    name: "Public".into(),
                    enabled: Some("True".into()),
                    allow_local_firewall_rules: Some("True".into()),
                    allow_inbound_rules: Some("True".into()),
                },
            ],
            rules,
        }
    }

    fn rule(
        name: &str,
        store: &str,
        program: Option<&str>,
        protocol: &str,
        direction: &str,
        action: &str,
    ) -> RuleFact {
        RuleFact {
            name: name.into(),
            display_name: Some("pc-remote-desktop.exe".into()),
            store: store.into(),
            policy_source_type: Some(
                if store == "PersistentStore" {
                    "Local"
                } else {
                    "Local"
                }
                .into(),
            ),
            program: program.map(str::to_string),
            protocol: Some(protocol.into()),
            direction: Some(direction.into()),
            action: Some(action.into()),
            enabled: Some("True".into()),
            profile: Some("Private, Public".into()),
            local_port: Some("Any".into()),
            remote_port: Some("Any".into()),
            local_address: Some("Any".into()),
            remote_address: Some("Any".into()),
            edge_traversal_policy: Some("Block".into()),
        }
    }

    fn generated_name(protocol: &str, executable: &str) -> String {
        format!("{protocol} Query User{{E135AA2D-EDD5-4E7B-AD5F-289009666C33}}{executable}")
    }

    fn effective_rule(name: &str, executable: &str) -> RuleFact {
        let (protocol, port) = if name == TCP_RULE_NAME {
            ("TCP", "43127")
        } else {
            ("UDP", "5353")
        };
        let mut rule = rule(
            name,
            "ActiveStore",
            Some(executable),
            protocol,
            "Inbound",
            "Allow",
        );
        rule.local_port = Some(port.into());
        rule.remote_address = Some("LocalSubnet".into());
        rule
    }

    #[derive(Default)]
    struct MockExecutor {
        state: Option<FirewallSnapshot>,
        calls: usize,
        cancel: bool,
    }

    impl PlanExecutor for MockExecutor {
        fn apply(&mut self, plan: &RepairPlan) -> Result<ExecutionResult, ()> {
            self.calls += 1;
            let state = self.state.as_mut().expect("test fixture state");
            if self.cancel {
                return Ok(ExecutionResult::Cancelled(Some(state.clone())));
            }
            for name in &plan.create_or_update {
                if !state
                    .rules
                    .iter()
                    .any(|r| r.store == "PersistentStore" && r.name == *name)
                {
                    let mut local = effective_rule(name, &plan.executable);
                    local.store = "PersistentStore".into();
                    state.rules.push(local);
                }
                if !state
                    .rules
                    .iter()
                    .any(|r| r.store == "ActiveStore" && r.name == *name)
                {
                    state.rules.push(effective_rule(name, &plan.executable));
                }
            }
            for name in &plan.disable_prompt_blocks {
                for rule in &mut state.rules {
                    if rule.name == *name && rule.store == "PersistentStore" {
                        rule.enabled = Some("False".into());
                    }
                    if rule.name == *name
                        && rule.store == "ActiveStore"
                        && is_local_policy_source(rule.policy_source_type.as_deref())
                    {
                        rule.enabled = Some("False".into());
                    }
                }
            }
            Ok(ExecutionResult::Configured(state.clone()))
        }
    }

    struct FalseSuccessExecutor(FirewallSnapshot);
    impl PlanExecutor for FalseSuccessExecutor {
        fn apply(&mut self, _plan: &RepairPlan) -> Result<ExecutionResult, ()> {
            Ok(ExecutionResult::Configured(self.0.clone()))
        }
    }

    #[test]
    fn repair_plan_matches_observed_query_user_name_and_leaves_named_admin_rules_alone() {
        let local = rule(
            &generated_name("TCP", exe()),
            "PersistentStore",
            Some(exe()),
            "TCP",
            "Inbound",
            "Block",
        );
        let mut near_miss = local.clone();
        near_miss.name = format!("TCP Query User{{not-a-guid}}{}", exe());
        let admin = rule(
            "Corp.App.Block",
            "PersistentStore",
            Some(exe()),
            "TCP",
            "Inbound",
            "Block",
        );
        let facts = snapshot(vec![local.clone(), near_miss, admin.clone()]);
        let plan = plan_repair(&facts, exe()).expect("valid local repair plan");
        assert_eq!(plan.disable_prompt_blocks, vec![local.name]);
        assert_eq!(plan.create_or_update, vec![TCP_RULE_NAME, UDP_RULE_NAME]);
        assert_eq!(
            facts
                .rules
                .iter()
                .find(|r| r.name == admin.name)
                .unwrap()
                .enabled
                .as_deref(),
            Some("True")
        );
    }

    #[test]
    fn generated_group_policy_block_is_reported_without_an_execution_attempt() {
        let mut managed = rule(
            &generated_name("TCP", exe()),
            "ActiveStore",
            Some(exe()),
            "TCP",
            "Inbound",
            "Block",
        );
        managed.policy_source_type = Some("GroupPolicy".into());
        let before = snapshot(vec![managed.clone()]);
        let mut executor = MockExecutor {
            state: Some(before.clone()),
            ..Default::default()
        };
        assert_eq!(
            run_repair_plan(&before, exe(), &mut executor),
            ExecutionResult::Restricted(Some(before))
        );
        assert_eq!(executor.calls, 0);
        assert_eq!(managed.enabled.as_deref(), Some("True"));
    }

    #[test]
    fn local_generated_rule_with_unexpected_display_name_is_ambiguous_and_untouched() {
        let mut ambiguous = rule(
            &generated_name("TCP", exe()),
            "PersistentStore",
            Some(exe()),
            "TCP",
            "Inbound",
            "Block",
        );
        ambiguous.display_name = Some("Unrelated rule".into());
        let before = snapshot(vec![ambiguous.clone()]);
        let mut executor = MockExecutor {
            state: Some(before.clone()),
            ..Default::default()
        };
        assert_eq!(
            run_repair_plan(&before, exe(), &mut executor),
            ExecutionResult::Restricted(Some(before))
        );
        assert_eq!(executor.calls, 0);
        assert_eq!(
            executor.state.unwrap().rules[0].enabled.as_deref(),
            Some("True")
        );
    }

    #[test]
    fn managed_profile_and_conflicting_owned_rule_prevent_any_mutation() {
        let mut managed_profile = snapshot(vec![]);
        managed_profile.profiles[1].allow_local_firewall_rules = Some("False".into());
        assert_eq!(
            plan_repair(&managed_profile, exe()),
            Err(PlanError::ManagedPolicy)
        );
        let conflict = rule(
            TCP_RULE_NAME,
            "PersistentStore",
            Some(r"C:\Other\app.exe"),
            "TCP",
            "Inbound",
            "Allow",
        );
        assert_eq!(
            plan_repair(&snapshot(vec![conflict]), exe()),
            Err(PlanError::AmbiguousOwnedRule)
        );
    }

    #[test]
    fn cancelled_or_failed_effective_readback_never_reports_configured() {
        let before = snapshot(vec![]);
        let mut cancelled = MockExecutor {
            state: Some(before.clone()),
            cancel: true,
            ..Default::default()
        };
        assert!(matches!(
            run_repair_plan(&before, exe(), &mut cancelled),
            ExecutionResult::Cancelled(_)
        ));
        let mut failed_readback = FalseSuccessExecutor(before.clone());
        assert_eq!(
            run_repair_plan(&before, exe(), &mut failed_readback),
            ExecutionResult::Restricted(Some(before))
        );
    }

    #[test]
    fn repeated_plan_application_is_idempotent_and_preserves_unrelated_blocks() {
        let prompt = rule(
            &generated_name("TCP", exe()),
            "PersistentStore",
            Some(exe()),
            "TCP",
            "Inbound",
            "Block",
        );
        let mut admin = rule(
            "Admin.Manual.Block",
            "PersistentStore",
            Some(exe()),
            "TCP",
            "Inbound",
            "Block",
        );
        admin.display_name = Some("Administrator rule".into());
        let mut group = rule(
            "Domain.Managed.Block",
            "ActiveStore",
            Some(exe()),
            "UDP",
            "Inbound",
            "Block",
        );
        group.policy_source_type = Some("GroupPolicy".into());
        group.program = Some(r"C:\Other\managed.exe".into());
        let before = snapshot(vec![prompt.clone(), admin.clone(), group.clone()]);
        let mut executor = MockExecutor {
            state: Some(before),
            ..Default::default()
        };
        for _ in 0..2 {
            let current = executor.state.as_ref().unwrap().clone();
            assert!(matches!(
                run_repair_plan(&current, exe(), &mut executor),
                ExecutionResult::Configured(_)
            ));
        }
        let final_state = executor.state.unwrap();
        assert_eq!(executor.calls, 2);
        for name in FIXED_RULE_NAMES {
            assert_eq!(
                final_state
                    .rules
                    .iter()
                    .filter(|r| r.store == "PersistentStore" && r.name == name)
                    .count(),
                1
            );
        }
        assert_eq!(
            final_state
                .rules
                .iter()
                .find(|r| r.name == prompt.name)
                .unwrap()
                .enabled
                .as_deref(),
            Some("False")
        );
        assert_eq!(
            final_state
                .rules
                .iter()
                .find(|r| r.name == admin.name)
                .unwrap()
                .enabled
                .as_deref(),
            Some("True")
        );
        assert_eq!(
            final_state
                .rules
                .iter()
                .find(|r| r.name == group.name)
                .unwrap()
                .enabled
                .as_deref(),
            Some("True")
        );
    }

    #[test]
    fn configured_message_only_claims_permissions_and_requests_a_retry() {
        assert!(REPAIR_MESSAGE.contains("Permissões locais configuradas"));
        assert!(REPAIR_MESSAGE.contains("Tente conectar novamente"));
        assert!(!REPAIR_MESSAGE.contains("conexão resolvida"));
    }
}
