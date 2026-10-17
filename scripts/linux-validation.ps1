$ErrorActionPreference = 'Stop'

$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$artifactRoot = Join-Path $repoRoot 'artifacts/linux-validation'
New-Item -ItemType Directory -Force -Path $artifactRoot | Out-Null
$dockerContext = Join-Path $artifactRoot 'docker-context'
New-Item -ItemType Directory -Force -Path $dockerContext | Out-Null
Copy-Item -Force (Join-Path $repoRoot 'Dockerfile.linux') (Join-Path $dockerContext 'Dockerfile.linux')

& docker build --tag pc-remote-linux-validation:local --file (Join-Path $dockerContext 'Dockerfile.linux') $dockerContext
if ($LASTEXITCODE -ne 0) {
    throw "Docker image build failed with exit code $LASTEXITCODE"
}

$mounts = @(
    "type=bind,source=$repoRoot,target=/source,readonly",
    "type=bind,source=$artifactRoot,target=/validation/evidence",
    'type=volume,source=pc-remote-linux-validation-cache,target=/validation/cache'
)
$dockerArgs = @(
    'run', '--rm', '--network', 'bridge', '--cap-drop', 'ALL',
    '--security-opt', 'no-new-privileges'
)
foreach ($mount in $mounts) {
    $dockerArgs += @('--mount', $mount)
}
$dockerArgs += @(
    '--env', 'CARGO_TARGET_DIR=/validation/cache/target',
    '--env', 'CARGO_HOME=/validation/cache/cargo',
    '--env', 'npm_config_cache=/validation/cache/npm',
    'pc-remote-linux-validation:local',
    'bash', '-c',
    'cp /source/scripts/linux-validation-container.sh /validation/driver.sh && exec bash /validation/driver.sh'
)

& docker @dockerArgs
if ($LASTEXITCODE -ne 0) {
    throw "Linux validation failed with exit code $LASTEXITCODE; see $artifactRoot"
}

Write-Host "Linux validation completed. Evidence: $artifactRoot"
