//! User-triggered, narrowly scoped Windows Firewall access repair.
//!
//! The generated Windows prompt-rule name is used only as a narrow recognition
//! heuristic alongside the local store, app path, display name, protocol,
//! direction, and block action. It is not proof of who created the rule.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

const TCP_RULE_NAME: &str = "PCRemote.LAN.TCP.v1";
const UDP_RULE_NAME: &str = "PCRemote.LAN.MDNS.v1";
const FIXED_RULE_NAMES: [&str; 2] = [TCP_RULE_NAME, UDP_RULE_NAME];
const REPAIR_MESSAGE: &str = "Permissões locais configuradas. Tente conectar novamente pelo celular.";
const RESTRICTED_MESSAGE: &str = "O Firewall do Windows ou uma política de rede ainda pode impedir a conexão.";
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
    path.replace('/', "\\").trim_end_matches('\\').to_lowercase()
}

fn value_is_true(value: Option<&str>) -> bool {
    matches!(value, Some(v) if v.eq_ignore_ascii_case("true") || v.eq_ignore_ascii_case("enabled"))
}

fn profile_disallows_local_rules(snapshot: &FirewallSnapshot) -> bool {
    snapshot.profiles.iter().any(|p| {
        matches!(p.name.as_str(), "Private" | "Public")
            && (p.allow_local_firewall_rules
                .as_deref()
                .is_some_and(|v| v.eq_ignore_ascii_case("false") || v.eq_ignore_ascii_case("disabled"))
                || p.allow_inbound_rules
                    .as_deref()
                    .is_some_and(|v| v.eq_ignore_ascii_case("false") || v.eq_ignore_ascii_case("disabled")))
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
        || !guid
            .chars()
            .enumerate()
            .all(|(i, c)| match i {
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
        && rule.display_name.as_deref().is_some_and(|display| {
            display.eq_ignore_ascii_case("PC Remote")
                || display.eq_ignore_ascii_case(expected_display)
        })
        && rule
            .program
            .as_deref()
            .is_some_and(|program| normalize_path(program) == normalize_path(executable))
        && rule.protocol.as_deref().is_some_and(|v| {
            v.eq_ignore_ascii_case(expected_protocol)
                || (expected_protocol == "TCP" && v == "6")
                || (expected_protocol == "UDP" && v == "17")
        })
        && rule.direction.as_deref().is_some_and(|v| v.eq_ignore_ascii_case("Inbound"))
        && rule.action.as_deref().is_some_and(|v| v.eq_ignore_ascii_case("Block"))
        && !rule
            .policy_source_type
            .as_deref()
            .is_some_and(|v| !v.eq_ignore_ascii_case("Local"))
}

fn target_rule_is_safe_to_manage(rule: &RuleFact, executable: &str) -> bool {
    rule.store.eq_ignore_ascii_case("PersistentStore")
        && rule
            .program
            .as_deref()
            .is_some_and(|program| normalize_path(program) == normalize_path(executable))
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
                    .is_some_and(|source| !source.eq_ignore_ascii_case("Local"))
            {
                return Err(PlanError::ManagedPolicy);
            }
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
    let expected_protocol = if expected == TCP_RULE_NAME { "TCP" } else { "UDP" };
    let expected_port = if expected == TCP_RULE_NAME { "43127" } else { "5353" };
    rule.store.eq_ignore_ascii_case("ActiveStore")
        && rule.name == expected
        && rule
            .program
            .as_deref()
            .is_some_and(|program| normalize_path(program) == normalize_path(executable))
        && rule.direction.as_deref().is_some_and(|v| v.eq_ignore_ascii_case("Inbound"))
        && rule.action.as_deref().is_some_and(|v| v.eq_ignore_ascii_case("Allow"))
        && value_is_true(rule.enabled.as_deref())
        && rule.protocol.as_deref().is_some_and(|v| {
            v.eq_ignore_ascii_case(expected_protocol)
                || (expected_protocol == "TCP" && v == "6")
                || (expected_protocol == "UDP" && v == "17")
        })
        && rule.local_port.as_deref().is_some_and(|v| v == expected_port)
        && rule.remote_port.as_deref().is_some_and(|v| v.eq_ignore_ascii_case("Any"))
        && rule.remote_address.as_deref().is_some_and(|v| v.eq_ignore_ascii_case("LocalSubnet"))
        && rule.local_address.as_deref().is_some_and(|v| v.eq_ignore_ascii_case("Any"))
        && rule.profile.as_deref().is_some_and(|v| {
            v.eq_ignore_ascii_case("Private, Public")
                || v.eq_ignore_ascii_case("Public, Private")
                || (v.to_ascii_lowercase().contains("private") && v.to_ascii_lowercase().contains("public"))
        })
        && rule.edge_traversal_policy.as_deref().is_some_and(|v| v.eq_ignore_ascii_case("Block"))
}

fn configured(snapshot: &FirewallSnapshot, executable: &str) -> bool {
    if profile_disallows_local_rules(snapshot) {
        return false;
    }
    let has_tcp = snapshot.rules.iter().any(|rule| {
        is_target_rule_effective(rule, TCP_RULE_NAME, executable)
    });
    let has_udp = snapshot.rules.iter().any(|rule| {
        is_target_rule_effective(rule, UDP_RULE_NAME, executable)
    });
    let has_enabled_prompt_block = snapshot.rules.iter().any(|rule| {
        rule.enabled.as_deref().is_some_and(|v| value_is_true(Some(v)))
            && is_recognized_local_prompt_block(rule, executable)
    });
    has_tcp && has_udp && !has_enabled_prompt_block
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
                (NetworkAccessState::Configured, "As permissões locais de rede estão configuradas.")
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
        Err(_) => return NetworkAccessStatus::windows(NetworkAccessState::Unknown, UNKNOWN_MESSAGE, None),
    };
    let executable = executable.to_string_lossy().to_string();
    let (before, _) = match windows_snapshot_for(&executable) {
        Ok(value) => value,
        Err(()) => return NetworkAccessStatus::windows(NetworkAccessState::Unknown, UNKNOWN_MESSAGE, None),
    };
    let profile = profile_label(&before);
    let plan = match plan_repair(&before, &executable) {
        Ok(plan) => plan,
        Err(PlanError::ManagedPolicy | PlanError::AmbiguousOwnedRule) => {
            return NetworkAccessStatus::windows(NetworkAccessState::Restricted, RESTRICTED_MESSAGE, profile)
        }
    };

    match execute_windows_plan(&plan) {
        Ok(ExecutionResult::Configured(snapshot)) if configured(&snapshot, &executable) => {
            NetworkAccessStatus::windows(NetworkAccessState::Configured, REPAIR_MESSAGE, profile_label(&snapshot))
        }
        Ok(ExecutionResult::Cancelled(snapshot)) => NetworkAccessStatus::windows(
            NetworkAccessState::Cancelled,
            "A solicitação de administrador foi cancelada. Nenhuma permissão foi alterada.",
            snapshot.and_then(|facts| profile_label(&facts)).or(profile),
        ),
        Ok(ExecutionResult::Restricted(snapshot)) => NetworkAccessStatus::windows(
            NetworkAccessState::Restricted,
            RESTRICTED_MESSAGE,
            snapshot.and_then(|facts| profile_label(&facts)).or(profile),
        ),
        Ok(ExecutionResult::Configured(_)) | Err(()) => {
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

#[cfg(windows)]
#[derive(Clone, Debug)]
enum ExecutionResult {
    Configured(FirewallSnapshot),
    Restricted(Option<FirewallSnapshot>),
    Cancelled(Option<FirewallSnapshot>),
}

#[cfg(windows)]
fn execute_windows_plan(plan: &RepairPlan) -> Result<ExecutionResult, ()> {
    let executable_literal = ps_literal(&plan.executable);
    let mut disable_names = plan.disable_prompt_blocks.clone();
    disable_names.sort();
    disable_names.dedup();
    let disable_literals = disable_names.iter().map(|s| ps_literal(s)).collect::<Vec<_>>().join(", ");
    let script = format!(
        r#"
$ErrorActionPreference = 'Stop'
$exe = {executable_literal}
$authorizedPromptNames = @({disable_literals})
$names = @('{tcp_name}', '{udp_name}')
function Read-Snapshot {{
  $activeProfiles = @()
  try {{
    $activeProfiles = @((Get-NetConnectionProfile -ErrorAction Stop | ForEach-Object {{ [string]$_.NetworkCategory }} | Sort-Object -Unique))
  }} catch {{ $activeProfiles = @() }}
  $profiles = @((Get-NetFirewallProfile -PolicyStore ActiveStore -ErrorAction Stop | ForEach-Object {{
    [pscustomobject]@{{ name=[string]$_.Name; enabled=[string]$_.Enabled; allowLocalFirewallRules=[string]$_.AllowLocalFirewallRules; allowInboundRules=[string]$_.AllowInboundRules }}
  }}))
  $rules = @()
  foreach ($store in @('PersistentStore','ActiveStore')) {{
    $found = @(Get-NetFirewallRule -PolicyStore $store -ErrorAction Stop | Where-Object {{
      $_.Name -in $names -or $_.Name -match '^(TCP|UDP) Query User\{{[0-9A-Fa-f-]{{36}}\}}'
    }})
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
  return [pscustomobject]@{{ activeProfiles=$activeProfiles; profiles=$profiles; rules=$rules }}
}}
function Emit($state, $message, $snapshot) {{
  $profile = $null
  if ($null -ne $snapshot -and @($snapshot.activeProfiles).Count -gt 0) {{ $profile = (@($snapshot.activeProfiles) -join ', ') }}
  $result = [pscustomobject]@{{ supported=$true; state=$state; message=$message; profile=$profile }}
  [Console]::OutputEncoding = New-Object System.Text.UTF8Encoding($false)
  [Console]::Out.WriteLine((ConvertTo-Json -InputObject $result -Compress -Depth 5))
}}
function Test-IsAdmin {{
  $identity = [Security.Principal.WindowsIdentity]::GetCurrent()
  $principal = New-Object Security.Principal.WindowsPrincipal($identity)
  return $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
}}
function Apply-Repair {{
  # Validate every same-name rule before changing anything.
  $snapshot = Read-Snapshot
  foreach ($name in $names) {{
    $local = @($snapshot.rules | Where-Object {{ $_.store -eq 'PersistentStore' -and $_.name -eq $name }})
    foreach ($rule in $local) {{
      if ([string]::IsNullOrWhiteSpace($rule.program) -or -not [string]::Equals($rule.program, $exe, [StringComparison]::OrdinalIgnoreCase)) {{ return $false }}
    }}
    $active = @($snapshot.rules | Where-Object {{ $_.store -eq 'ActiveStore' -and $_.name -eq $name }})
    foreach ($rule in $active) {{
      if ($rule.policySourceType -and $rule.policySourceType -ne 'Local') {{ return $false }}
      if ([string]::IsNullOrWhiteSpace($rule.program) -or -not [string]::Equals($rule.program, $exe, [StringComparison]::OrdinalIgnoreCase)) {{ return $false }}
    }}
  }}
  foreach ($name in $authorizedPromptNames) {{
    $rule = Get-NetFirewallRule -PolicyStore PersistentStore -Name $name -ErrorAction SilentlyContinue
    if ($null -eq $rule) {{ return $false }}
    $app = @(Get-NetFirewallApplicationFilter -AssociatedNetFirewallRule $rule -ErrorAction SilentlyContinue)
    $port = @(Get-NetFirewallPortFilter -AssociatedNetFirewallRule $rule -ErrorAction SilentlyContinue)
    $matches = $name -match '^(TCP|UDP) Query User\{{[0-9A-Fa-f-]{{36}}\}}' + [regex]::Escape($exe) + '$'
    $protocol = $Matches[1]
    $expectedDisplay = [IO.Path]::GetFileName($exe)
    if (-not $matches -or $app.Count -ne 1 -or $port.Count -ne 1 -or
        -not [string]::Equals($app[0].Program, $exe, [StringComparison]::OrdinalIgnoreCase) -or
        -not ($rule.DisplayName -ieq 'PC Remote' -or $rule.DisplayName -ieq $expectedDisplay) -or
        -not ($rule.Direction -eq 'Inbound') -or -not ($rule.Action -eq 'Block') -or
        -not (($protocol -eq 'TCP' -and ($port[0].Protocol -eq 'TCP' -or $port[0].Protocol -eq 6)) -or ($protocol -eq 'UDP' -and ($port[0].Protocol -eq 'UDP' -or $port[0].Protocol -eq 17))) {{ return $false }}
    Disable-NetFirewallRule -InputObject $rule -ErrorAction Stop | Out-Null
  }}
  $tcp = Get-NetFirewallRule -PolicyStore PersistentStore -Name '{tcp_name}' -ErrorAction SilentlyContinue
  if ($null -eq $tcp) {{
    New-NetFirewallRule -PolicyStore PersistentStore -Name '{tcp_name}' -DisplayName 'PC Remote (TCP LAN)' -Program $exe -Direction Inbound -Action Allow -Protocol TCP -LocalPort 43127 -RemoteAddress LocalSubnet -Profile Private,Public -EdgeTraversalPolicy Block -Enabled True -ErrorAction Stop | Out-Null
  }} else {{
    Set-NetFirewallRule -InputObject $tcp -Direction Inbound -Action Allow -Profile Private,Public -EdgeTraversalPolicy Block -Enabled True -ErrorAction Stop | Out-Null
    Set-NetFirewallApplicationFilter -AssociatedNetFirewallRule $tcp -Program $exe -ErrorAction Stop | Out-Null
    Set-NetFirewallPortFilter -AssociatedNetFirewallRule $tcp -Protocol TCP -LocalPort 43127 -RemotePort Any -ErrorAction Stop | Out-Null
    Set-NetFirewallAddressFilter -AssociatedNetFirewallRule $tcp -LocalAddress Any -RemoteAddress LocalSubnet -ErrorAction Stop | Out-Null
  }}
  $udp = Get-NetFirewallRule -PolicyStore PersistentStore -Name '{udp_name}' -ErrorAction SilentlyContinue
  if ($null -eq $udp) {{
    New-NetFirewallRule -PolicyStore PersistentStore -Name '{udp_name}' -DisplayName 'PC Remote (mDNS LAN)' -Program $exe -Direction Inbound -Action Allow -Protocol UDP -LocalPort 5353 -RemoteAddress LocalSubnet -Profile Private,Public -EdgeTraversalPolicy Block -Enabled True -ErrorAction Stop | Out-Null
  }} else {{
    Set-NetFirewallRule -InputObject $udp -Direction Inbound -Action Allow -Profile Private,Public -EdgeTraversalPolicy Block -Enabled True -ErrorAction Stop | Out-Null
    Set-NetFirewallApplicationFilter -AssociatedNetFirewallRule $udp -Program $exe -ErrorAction Stop | Out-Null
    Set-NetFirewallPortFilter -AssociatedNetFirewallRule $udp -Protocol UDP -LocalPort 5353 -RemotePort Any -ErrorAction Stop | Out-Null
    Set-NetFirewallAddressFilter -AssociatedNetFirewallRule $udp -LocalAddress Any -RemoteAddress LocalSubnet -ErrorAction Stop | Out-Null
  }}
  return $true
}}
try {{
  if (Test-IsAdmin) {{
    if (-not (Apply-Repair)) {{ Emit 'restricted' '{restricted}' (Read-Snapshot); exit 2 }}
    $snapshot = Read-Snapshot
    # The Rust side independently validates the effective readback before returning configured.
    Emit 'configured' '{configured}' $snapshot
    exit 0
  }}
  $self = (Get-Process -Id $PID).Path
  $encodedChild = '{child_encoded}'
  try {{
    $child = Start-Process -FilePath $self -ArgumentList "-NoProfile -NonInteractive -EncodedCommand $encodedChild" -Verb RunAs -WindowStyle Hidden -PassThru -Wait
  }} catch {{
    $native = 0
    try {{ $native = [int]$_.Exception.NativeErrorCode }} catch {{}}
    $hresult = ([int]$_.Exception.HResult) -band 0xffff
    if ($native -eq 1223 -or $hresult -eq 1223) {{ Emit 'cancelled' 'A solicitação de administrador foi cancelada. Nenhuma permissão foi alterada.' $null; exit 0 }}
    Emit 'unknown' '{unknown}' $null; exit 3
  }}
  $snapshot = Read-Snapshot
  if ($child.ExitCode -eq 0) {{ Emit 'configured' '{configured}' $snapshot; exit 0 }}
  Emit 'restricted' '{restricted}' $snapshot
  exit 2
}} catch {{
  Emit 'unknown' '{unknown}' $null
  exit 3
}}
"#,
        executable_literal = executable_literal,
        disable_literals = disable_literals,
        tcp_name = TCP_RULE_NAME,
        udp_name = UDP_RULE_NAME,
        configured = REPAIR_MESSAGE,
        restricted = RESTRICTED_MESSAGE,
        unknown = UNKNOWN_MESSAGE,
        child_encoded = encode_powershell_utf16(&apply_script(&plan.executable, &disable_names)),
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
            let snapshot = windows_snapshot_for(&plan.executable).ok().map(|(snapshot, _)| snapshot);
            Ok(ExecutionResult::Restricted(snapshot))
        }
        NetworkAccessState::Unknown => Err(()),
    }
}

#[cfg(windows)]
fn apply_script(executable: &str, authorized_names: &[String]) -> String {
    let executable = ps_literal(executable);
    let names = authorized_names.iter().map(|s| ps_literal(s)).collect::<Vec<_>>().join(", ");
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
    $found = @(Get-NetFirewallRule -PolicyStore $store -ErrorAction Stop | Where-Object {{ $_.Name -in @('{tcp_name}','{udp_name}') -or $_.Name -match '^(TCP|UDP) Query User\{{[0-9A-Fa-f-]{{36}}\}}' }})
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
  foreach ($name in @('{tcp_name}','{udp_name}')) {{
    $local = @($snapshot.rules | Where-Object {{ $_.store -eq 'PersistentStore' -and $_.name -eq $name }})
    foreach ($rule in $local) {{ if ([string]::IsNullOrWhiteSpace($rule.program) -or -not [string]::Equals($rule.program,$exe,[StringComparison]::OrdinalIgnoreCase)) {{ return $false }} }}
    $active = @($snapshot.rules | Where-Object {{ $_.store -eq 'ActiveStore' -and $_.name -eq $name }})
    foreach ($rule in $active) {{ if (($rule.policySourceType -and $rule.policySourceType -ne 'Local') -or [string]::IsNullOrWhiteSpace($rule.program) -or -not [string]::Equals($rule.program,$exe,[StringComparison]::OrdinalIgnoreCase)) {{ return $false }} }}
  }}
  foreach ($name in $authorizedPromptNames) {{
    $rule = Get-NetFirewallRule -PolicyStore PersistentStore -Name $name -ErrorAction SilentlyContinue
    if ($null -eq $rule) {{ return $false }}
    $app = @(Get-NetFirewallApplicationFilter -AssociatedNetFirewallRule $rule -ErrorAction SilentlyContinue)
    $port = @(Get-NetFirewallPortFilter -AssociatedNetFirewallRule $rule -ErrorAction SilentlyContinue)
    if ($app.Count -ne 1 -or $port.Count -ne 1 -or -not [string]::Equals($app[0].Program,$exe,[StringComparison]::OrdinalIgnoreCase) -or $rule.Direction -ne 'Inbound' -or $rule.Action -ne 'Block') {{ return $false }}
    Disable-NetFirewallRule -InputObject $rule -ErrorAction Stop | Out-Null
  }}
  $tcp = Get-NetFirewallRule -PolicyStore PersistentStore -Name '{tcp_name}' -ErrorAction SilentlyContinue
  if ($null -eq $tcp) {{ New-NetFirewallRule -PolicyStore PersistentStore -Name '{tcp_name}' -DisplayName 'PC Remote (TCP LAN)' -Program $exe -Direction Inbound -Action Allow -Protocol TCP -LocalPort 43127 -RemoteAddress LocalSubnet -Profile Private,Public -EdgeTraversalPolicy Block -Enabled True -ErrorAction Stop | Out-Null }} else {{
    Set-NetFirewallRule -InputObject $tcp -Direction Inbound -Action Allow -Profile Private,Public -EdgeTraversalPolicy Block -Enabled True -ErrorAction Stop | Out-Null
    Set-NetFirewallApplicationFilter -AssociatedNetFirewallRule $tcp -Program $exe -ErrorAction Stop | Out-Null
    Set-NetFirewallPortFilter -AssociatedNetFirewallRule $tcp -Protocol TCP -LocalPort 43127 -RemotePort Any -ErrorAction Stop | Out-Null
    Set-NetFirewallAddressFilter -AssociatedNetFirewallRule $tcp -LocalAddress Any -RemoteAddress LocalSubnet -ErrorAction Stop | Out-Null
  }}
  $udp = Get-NetFirewallRule -PolicyStore PersistentStore -Name '{udp_name}' -ErrorAction SilentlyContinue
  if ($null -eq $udp) {{ New-NetFirewallRule -PolicyStore PersistentStore -Name '{udp_name}' -DisplayName 'PC Remote (mDNS LAN)' -Program $exe -Direction Inbound -Action Allow -Protocol UDP -LocalPort 5353 -RemoteAddress LocalSubnet -Profile Private,Public -EdgeTraversalPolicy Block -Enabled True -ErrorAction Stop | Out-Null }} else {{
    Set-NetFirewallRule -InputObject $udp -Direction Inbound -Action Allow -Profile Private,Public -EdgeTraversalPolicy Block -Enabled True -ErrorAction Stop | Out-Null
    Set-NetFirewallApplicationFilter -AssociatedNetFirewallRule $udp -Program $exe -ErrorAction Stop | Out-Null
    Set-NetFirewallPortFilter -AssociatedNetFirewallRule $udp -Protocol UDP -LocalPort 5353 -RemotePort Any -ErrorAction Stop | Out-Null
    Set-NetFirewallAddressFilter -AssociatedNetFirewallRule $udp -LocalAddress Any -RemoteAddress LocalSubnet -ErrorAction Stop | Out-Null
  }}
  return $true
}}
try {{
  if (-not (Test-IsAdmin)) {{ exit 4 }}
  if (-not (Apply-Repair)) {{ exit 2 }}
  exit 0
}} catch {{ exit 3 }}
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
  $found = @(Get-NetFirewallRule -PolicyStore $store -ErrorAction Stop | Where-Object {{ $_.Name -in $names -or $_.Name -match '^(TCP|UDP) Query User\{{[0-9A-Fa-f-]{{36}}\}}' }})
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
    let bytes = script.encode_utf16().flat_map(u16::to_le_bytes).collect::<Vec<_>>();
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

#[cfg(windows)]
fn run_powershell(script: &str) -> Result<String, ()> {
    use std::{os::windows::process::CommandExt, process::Command};
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let executable = powershell_path()?;
    let encoded = encode_powershell_utf16(script);
    let output = Command::new(executable)
        .args(["-NoProfile", "-NonInteractive", "-EncodedCommand", &encoded])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map_err(|_| ())?;
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

    fn exe() -> &'static str {
        r"C:\Program Files\PC Remote\pc-remote-desktop.exe"
    }

    fn snapshot(rules: Vec<RuleFact>) -> FirewallSnapshot {
        FirewallSnapshot {
            active_profiles: vec!["Private".into()],
            profiles: vec![
                ProfileFact { name: "Private".into(), enabled: Some("True".into()), allow_local_firewall_rules: Some("True".into()), allow_inbound_rules: Some("True".into()) },
                ProfileFact { name: "Public".into(), enabled: Some("True".into()), allow_local_firewall_rules: Some("True".into()), allow_inbound_rules: Some("True".into()) },
            ],
            rules,
        }
    }

    fn rule(name: &str, store: &str, program: Option<&str>, protocol: &str, direction: &str, action: &str) -> RuleFact {
        RuleFact {
            name: name.into(), display_name: Some("pc-remote-desktop.exe".into()), store: store.into(),
            policy_source_type: Some(if store == "PersistentStore" { "Local" } else { "Local" }.into()),
            program: program.map(str::to_string), protocol: Some(protocol.into()), direction: Some(direction.into()), action: Some(action.into()),
            enabled: Some("True".into()), profile: Some("Private, Public".into()), local_port: Some("Any".into()), remote_port: Some("Any".into()),
            local_address: Some("Any".into()), remote_address: Some("Any".into()), edge_traversal_policy: Some("Block".into()),
        }
    }

    fn generated_name(protocol: &str, exe: &str) -> String {
        format!("{protocol} Query User{{E135AA2D-EDD5-4E7B-AD5F-289009666C33}}{exe}")
    }

    #[test]
    fn repair_plan_disables_only_exact_local_generated_blocks_and_keeps_policy_blocks() {
        let local = rule(&generated_name("TCP", exe()), "PersistentStore", Some(exe()), "TCP", "Inbound", "Block");
        let mut group_policy = local.clone();
        group_policy.store = "ActiveStore".into();
        group_policy.policy_source_type = Some("GroupPolicy".into());
        let mut wrong_program = local.clone();
        wrong_program.name = generated_name("UDP", r"C:\Other\app.exe");
        let mut wrong_protocol = local.clone();
        wrong_protocol.protocol = Some("UDP".into());
        let facts = snapshot(vec![local.clone(), group_policy, wrong_program, wrong_protocol]);
        let plan = plan_repair(&facts, exe()).expect("valid local repair plan");
        assert_eq!(plan.executable, exe());
        assert_eq!(plan.disable_prompt_blocks, vec![local.name]);
        assert_eq!(plan.create_or_update, vec![TCP_RULE_NAME, UDP_RULE_NAME]);
    }

    #[test]
    fn managed_local_rule_policy_prevents_a_mutation_plan() {
        let mut facts = snapshot(vec![]);
        facts.profiles[1].allow_local_firewall_rules = Some("False".into());
        assert_eq!(plan_repair(&facts, exe()), Err(PlanError::ManagedPolicy));
    }

    #[test]
    fn ambiguous_owned_name_for_another_program_is_never_overwritten() {
        let conflict = rule(TCP_RULE_NAME, "PersistentStore", Some(r"C:\Other\app.exe"), "TCP", "Inbound", "Allow");
        assert_eq!(plan_repair(&snapshot(vec![conflict]), exe()), Err(PlanError::AmbiguousOwnedRule));
    }

    #[test]
    fn near_miss_prompt_name_or_admin_rule_is_not_disabled() {
        let mut no_guid = rule(&format!("TCP Query User{{not-a-guid}}{}", exe()), "PersistentStore", Some(exe()), "TCP", "Inbound", "Block");
        let admin = rule(&generated_name("TCP", exe()), "ActiveStore", Some(exe()), "TCP", "Inbound", "Block");
        let facts = snapshot(vec![no_guid.clone(), admin]);
        let plan = plan_repair(&facts, exe()).unwrap();
        assert!(plan.disable_prompt_blocks.is_empty());
        no_guid.name = generated_name("TCP", exe());
        no_guid.display_name = Some("Unrelated rule".into());
        assert!(!is_recognized_local_prompt_block(&no_guid, exe()));
    }

    #[test]
    fn success_requires_effective_readback_and_does_not_call_network_fixed() {
        let mut tcp = rule(TCP_RULE_NAME, "ActiveStore", Some(exe()), "TCP", "Inbound", "Allow");
        tcp.local_port = Some("43127".into());
        tcp.remote_address = Some("LocalSubnet".into());
        tcp.local_address = Some("Any".into());
        tcp.remote_port = Some("Any".into());
        let mut udp = rule(UDP_RULE_NAME, "ActiveStore", Some(exe()), "UDP", "Inbound", "Allow");
        udp.local_port = Some("5353".into());
        udp.remote_address = Some("LocalSubnet".into());
        udp.local_address = Some("Any".into());
        udp.remote_port = Some("Any".into());
        let facts = snapshot(vec![tcp, udp]);
        assert!(configured(&facts, exe()));
        assert!(REPAIR_MESSAGE.contains("Tente conectar novamente"));
        let mut incomplete = facts.clone();
        incomplete.rules.pop();
        assert!(!configured(&incomplete, exe()));
    }
}
