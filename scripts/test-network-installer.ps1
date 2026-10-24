param([string]$OutputDirectory = '')
$ErrorActionPreference = 'Stop'
$projectDirectory = Split-Path -Parent $PSScriptRoot
$hookPath = Join-Path $projectDirectory 'src-tauri\windows\network-hooks.nsh'
$fixtureDirectory = if ($OutputDirectory) { [System.IO.Path]::GetFullPath($OutputDirectory) } else { Join-Path $projectDirectory 'artifacts\qa\installer-hook' }
New-Item -ItemType Directory -Path $fixtureDirectory -Force | Out-Null
$compilerPath = Join-Path $env:WINDIR 'Microsoft.NET\Framework64\v4.0.30319\csc.exe'
$makensisPath = Join-Path $env:LOCALAPPDATA 'tauri\NSIS\makensis.exe'
$helperSourcePath = Join-Path $fixtureDirectory 'PermissionFixture.cs'
$helperPath = Join-Path $fixtureDirectory 'permission-fixture.exe'
$installerPath = Join-Path $fixtureDirectory 'installer-hook-test.exe'
$logPath = Join-Path $fixtureDirectory 'invocations.txt'
$source = @'
using System;
using System.IO;
class PermissionFixture {
    static int Main(string[] args) {
        File.AppendAllText(Environment.GetEnvironmentVariable("PCREMOTE_HOOK_TEST_LOG"),
            Environment.GetEnvironmentVariable("PCREMOTE_HOOK_TEST_CASE") + "|" + string.Join(" ", args) + Environment.NewLine);
        if (args.Length != 1 || args[0] != "--configure-network") return 97;
        return int.Parse(Environment.GetEnvironmentVariable("PCREMOTE_HOOK_TEST_EXIT"));
    }
}
'@
[System.IO.File]::WriteAllText($helperSourcePath, $source, (New-Object System.Text.UTF8Encoding($false)))
& $compilerPath /nologo /target:winexe "/out:$helperPath" $helperSourcePath
if ($LASTEXITCODE -ne 0) { throw 'Cannot compile harmless permission fixture.' }
$installerSource = @'
Unicode true
!include LogicLib.nsh
!define MAINBINARYNAME "permission-fixture"
!include "__HOOK__"
Name "PC Remote network hook test"
OutFile "__OUTPUT__"
InstallDir "__INSTALL_DIR__"
RequestExecutionLevel user
SilentInstall silent
Section
  !insertmacro NSIS_HOOK_POSTINSTALL
  ${If} ${Errors}
    SetErrorLevel 98
  ${EndIf}
SectionEnd
'@
$installerSource = $installerSource.Replace('__HOOK__', $hookPath).Replace('__OUTPUT__', $installerPath).Replace('__INSTALL_DIR__', $fixtureDirectory)
$installerSourcePath = Join-Path $fixtureDirectory 'installer-hook-test.nsi'
[System.IO.File]::WriteAllText($installerSourcePath, $installerSource, (New-Object System.Text.UTF8Encoding($true)))
& $makensisPath /V2 $installerSourcePath
if ($LASTEXITCODE -ne 0) { throw 'Cannot compile production NSIS hook.' }
[System.IO.File]::WriteAllText($logPath, '', (New-Object System.Text.UTF8Encoding($false)))
$env:PCREMOTE_HOOK_TEST_LOG = $logPath
$results = @()
foreach ($case in @(
  @{ Name='install'; Code=0; Arguments='/S' },
  @{ Name='update'; Code=0; Arguments='/S /UPDATE /R' },
  @{ Name='cancel'; Code=2; Arguments='/S /UPDATE /R' },
  @{ Name='restricted'; Code=3; Arguments='/S' }
)) {
  $env:PCREMOTE_HOOK_TEST_CASE = $case.Name
  $env:PCREMOTE_HOOK_TEST_EXIT = [string]$case.Code
  $startedAt = [DateTimeOffset]::UtcNow
  $installerProcess = Start-Process -FilePath $installerPath -ArgumentList $case.Arguments -PassThru -WindowStyle Hidden
  if (-not $installerProcess.WaitForExit(15000)) {
    $installerProcess.Kill()
    throw "Hook test timed out: $($case.Name)"
  }
  $matchesForCase = @(Get-Content -LiteralPath $logPath | Where-Object { $_ -eq ($case.Name + '|--configure-network') })
  if ($matchesForCase.Count -ne 1 -or $installerProcess.ExitCode -ne 0) { throw "Hook invocation failed: $($case.Name)" }
  $results += [pscustomobject]@{ case=$case.Name; helperExit=$case.Code; installerExit=$installerProcess.ExitCode; calls=$matchesForCase.Count; startedAtUtc=$startedAt.ToString('o') }
}
$missingInstallerPath = Join-Path $fixtureDirectory 'installer-hook-missing-test.exe'
$missingHelperPath = Join-Path $fixtureDirectory 'missing-permission-fixture.exe'
if (Test-Path -LiteralPath $missingHelperPath) { throw 'Unexpected missing-helper fixture already exists.' }
$missingSource = $installerSource.Replace('!define MAINBINARYNAME "permission-fixture"', '!define MAINBINARYNAME "missing-permission-fixture"').Replace($installerPath, $missingInstallerPath)
$missingSourcePath = Join-Path $fixtureDirectory 'installer-hook-missing-test.nsi'
[System.IO.File]::WriteAllText($missingSourcePath, $missingSource, (New-Object System.Text.UTF8Encoding($true)))
& $makensisPath /V2 $missingSourcePath
if ($LASTEXITCODE -ne 0) { throw 'Cannot compile missing-helper hook fixture.' }
$callsBefore = @(Get-Content -LiteralPath $logPath).Count
$missingProcess = Start-Process -FilePath $missingInstallerPath -ArgumentList '/S /UPDATE /R' -PassThru -WindowStyle Hidden
if (-not $missingProcess.WaitForExit(15000)) { $missingProcess.Kill(); throw 'Missing-helper test timed out.' }
$extraCalls = @(Get-Content -LiteralPath $logPath).Count - $callsBefore
if ($missingProcess.ExitCode -ne 0 -or $extraCalls -ne 0) { throw 'Missing helper left the installation in an error state.' }
$results += [pscustomobject]@{ case='launch-failed'; helperExit=$null; installerExit=$missingProcess.ExitCode; calls=$extraCalls; startedAtUtc=[DateTimeOffset]::UtcNow.ToString('o') }
$report = [pscustomobject]@{ productionHook=$hookPath; fixture='Harmless fake CLI: no UAC, firewall commands, app startup, or network access.'; results=$results }
$report | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath (Join-Path $fixtureDirectory 'results.json') -Encoding UTF8
$results | Format-Table
