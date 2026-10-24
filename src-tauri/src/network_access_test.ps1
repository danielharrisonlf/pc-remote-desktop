# Test-only NetSecurity substitutes. No Windows settings or real rules are read
# or modified. Advanced functions deliberately reject unsupported parameters.
$script:rules = @{}
$script:mutations = New-Object 'System.Collections.Generic.List[string]'
$script:failFilter = $false
$script:managedProfile = $false
function Assert($condition, [string]$message) { if (-not $condition) { throw $message } }
function Fixture-Rule($name, $protocol, $action, $program = $exe) {
  [pscustomobject]@{ Name=$name; DisplayName='PC Remote'; Program=$program; Protocol=$protocol; LocalPort='Any'; RemotePort='Any'; LocalAddress='Any'; RemoteAddress='Any'; Profile='Private, Public'; Direction='Inbound'; Action=$action; Enabled='True'; PolicyStoreSourceType='Local'; EdgeTraversalPolicy='Block'; ActiveOnly=$false }
}
function Get-NetConnectionProfile { [CmdletBinding()]param(); [pscustomobject]@{ NetworkCategory='Private' } }
function Get-NetFirewallProfile {
  [CmdletBinding()]param([string]$PolicyStore)
  foreach ($name in @('Private','Public')) { [pscustomobject]@{ Name=$name; Enabled='True'; AllowLocalFirewallRules=if ($script:managedProfile) {'False'} else {'True'}; AllowInboundRules='True' } }
}
function Get-NetFirewallRule {
  [CmdletBinding()]param([string]$PolicyStore, [string]$Name)
  foreach ($rule in @($script:rules.Values)) {
    if ($PolicyStore -eq 'PersistentStore' -and $rule.ActiveOnly) { continue }
    if (-not $Name -or $rule.Name -eq $Name) { $rule }
  }
}
function Get-NetFirewallApplicationFilter { [CmdletBinding()]param($AssociatedNetFirewallRule); $AssociatedNetFirewallRule }
function Get-NetFirewallPortFilter { [CmdletBinding()]param($AssociatedNetFirewallRule); $AssociatedNetFirewallRule }
function Get-NetFirewallAddressFilter { [CmdletBinding()]param($AssociatedNetFirewallRule); $AssociatedNetFirewallRule }
function New-NetFirewallRule {
  [CmdletBinding()]param([string]$PolicyStore,[string]$Name,[string]$DisplayName,[string]$Program,[string]$Direction,[string]$Action,[string]$Protocol,[string]$LocalPort,[string]$RemoteAddress,[string[]]$Profile,[string]$EdgeTraversalPolicy,[string]$Enabled)
  Assert (-not $script:rules.ContainsKey($Name)) 'duplicate rule'
  Assert ($PolicyStore -eq 'PersistentStore') 'wrong store'
  $rule = Fixture-Rule $Name $Protocol $Action $Program
  foreach ($key in @('DisplayName','Direction','LocalPort','RemoteAddress','EdgeTraversalPolicy','Enabled')) { $rule.$key = $PSBoundParameters[$key] }
  $rule.Profile = $Profile -join ', '
  $script:rules[$Name] = $rule
  $script:mutations.Add("create:$Name")
}
function Set-NetFirewallRule {
  [CmdletBinding()]param($InputObject,[string]$Direction,[string]$Action,[string[]]$Profile,[string]$EdgeTraversalPolicy,[string]$Enabled)
  foreach ($key in @('Direction','Action','EdgeTraversalPolicy','Enabled')) { if ($PSBoundParameters.ContainsKey($key)) { $InputObject.$key=$PSBoundParameters[$key] } }
  if ($PSBoundParameters.ContainsKey('Profile')) { $InputObject.Profile=$Profile -join ', ' }
  $script:mutations.Add("rule:$($InputObject.Name):$Enabled")
}
function Set-NetFirewallApplicationFilter {
  [CmdletBinding()]param([Parameter(ValueFromPipeline=$true)]$InputObject,[string]$Program)
  process {
    Assert ($InputObject.Enabled -eq 'False') 'must disable before changing program'
    if ($script:failFilter) { throw 'injected filter failure' }
    $InputObject.Program=$Program
    $script:mutations.Add("app:$($InputObject.Name)")
  }
}
function Set-NetFirewallPortFilter {
  [CmdletBinding()]param([Parameter(ValueFromPipeline=$true)]$InputObject,[string]$Protocol,[string]$LocalPort,[string]$RemotePort)
  process {
    Assert ($InputObject.Enabled -eq 'False') 'must disable before changing ports'
    $InputObject.Protocol=$Protocol; $InputObject.LocalPort=$LocalPort; $InputObject.RemotePort=$RemotePort
    $script:mutations.Add("port:$($InputObject.Name)")
  }
}
function Set-NetFirewallAddressFilter {
  [CmdletBinding()]param([Parameter(ValueFromPipeline=$true)]$InputObject,[string]$LocalAddress,[string]$RemoteAddress)
  process {
    Assert ($InputObject.Enabled -eq 'False') 'must disable before changing addresses'
    $InputObject.LocalAddress=$LocalAddress; $InputObject.RemoteAddress=$RemoteAddress
    $script:mutations.Add("address:$($InputObject.Name)")
  }
}
function Disable-NetFirewallRule { [CmdletBinding()]param($InputObject); $InputObject.Enabled='False'; $script:mutations.Add("disable:$($InputObject.Name)") }

for ($index=0; $index -lt $authorizedPromptNames.Count; $index++) {
  $name=$authorizedPromptNames[$index]
  $protocol=if ($index -eq 0) {'TCP'} else {'UDP'}
  $script:rules[$name] = Fixture-Rule $name $protocol 'Block'
}
$script:rules['Admin.Manual.Block'] = Fixture-Rule 'Admin.Manual.Block' 'TCP' 'Block' 'C:\Other\admin.exe'
$managed = Fixture-Rule 'Domain.Managed.Block' 'UDP' 'Block' 'C:\Other\managed.exe'
$managed.ActiveOnly=$true; $managed.PolicyStoreSourceType='GroupPolicy'
$script:rules[$managed.Name]=$managed
Assert (Apply-Repair) 'initial production repair failed'
Assert (Apply-Repair) 'repeated production repair failed'
Assert ($script:rules.Count -eq 6) 'unexpected rule count'
foreach ($name in $authorizedPromptNames) { Assert ($script:rules[$name].Enabled -eq 'False') 'prompt was not disabled' }
foreach ($name in @('Admin.Manual.Block','Domain.Managed.Block')) { Assert ($script:rules[$name].Enabled -eq 'True') 'unrelated rule changed' }
foreach ($name in @('PCRemote.LAN.TCP.v1','PCRemote.LAN.MDNS.v1')) {
  $rule=$script:rules[$name]
  Assert ($rule.Enabled -eq 'True' -and $rule.Program -ceq $exe -and $rule.RemoteAddress -eq 'LocalSubnet' -and $rule.Profile -eq 'Private, Public' -and $rule.RemotePort -eq 'Any' -and $rule.LocalAddress -eq 'Any') 'incorrect rule scope'
}
Assert ($script:rules['PCRemote.LAN.TCP.v1'].LocalPort -eq '43127') 'incorrect TCP port'
Assert ($script:rules['PCRemote.LAN.MDNS.v1'].LocalPort -eq '5353') 'incorrect UDP port'
# A setter error leaves the owned rule disabled, with prompt blocks untouched.
$script:failFilter=$true
$failed=$false
try { $null=Apply-Repair } catch { $failed=$true }
Assert $failed 'filter failure was swallowed'
Assert ($script:rules['PCRemote.LAN.TCP.v1'].Enabled -eq 'False') 'failure enabled partially scoped rule'
$script:failFilter=$false
Assert (Apply-Repair) 'repair after injected failure failed'
# Revalidate policy and identity immediately before any mutation.
$count=$script:mutations.Count
$script:managedProfile=$true
Assert (-not (Apply-Repair)) 'managed profile accepted'
Assert ($script:mutations.Count -eq $count) 'managed profile mutated'
$script:managedProfile=$false
$script:rules['PCRemote.LAN.TCP.v1'].Program='C:\Other\conflict.exe'
Assert (-not (Apply-Repair)) 'conflicting identity accepted'
Assert ($script:mutations.Count -eq $count) 'conflicting identity mutated'
$script:rules['PCRemote.LAN.TCP.v1'].Program=$exe
$prompt=$script:rules[$authorizedPromptNames[0]]
$prompt.Enabled='True'; $prompt.PolicyStoreSourceType='GroupPolicy'; $prompt.ActiveOnly=$true
Assert (-not (Apply-Repair)) 'managed prompt accepted'
Assert ($script:mutations.Count -eq $count) 'managed prompt mutated'
$prompt.ActiveOnly=$false; $prompt.PolicyStoreSourceType='Local'; $prompt.DisplayName='Administrator custom rule'
Assert (-not (Apply-Repair)) 'ambiguous prompt accepted'
Assert ($script:mutations.Count -eq $count) 'ambiguous prompt mutated'
[Console]::Out.Write('passed')
