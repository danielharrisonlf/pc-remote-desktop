param(
    [ValidateSet('22.04', '24.04', '26.04')]
    [string]$UbuntuVersion = '22.04'
)
$ErrorActionPreference = 'Stop'

$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$artifactName = if ($UbuntuVersion -eq '22.04') { 'artifacts/linux-validation' } else { "artifacts/linux-validation-$UbuntuVersion" }
$artifactRoot = Join-Path $repoRoot $artifactName
$imageName = "pc-remote-linux-validation:$UbuntuVersion"
$cacheName = if ($UbuntuVersion -eq '22.04') { 'pc-remote-linux-validation-cache' } else { "pc-remote-linux-validation-cache-$UbuntuVersion" }
New-Item -ItemType Directory -Force -Path $artifactRoot | Out-Null
$dockerContext = Join-Path $artifactRoot 'docker-context'
New-Item -ItemType Directory -Force -Path $dockerContext | Out-Null
Copy-Item -Force (Join-Path $repoRoot 'Dockerfile.linux') (Join-Path $dockerContext 'Dockerfile.linux')

& docker build --build-arg "UBUNTU_VERSION=$UbuntuVersion" --tag $imageName --file (Join-Path $dockerContext 'Dockerfile.linux') $dockerContext
if ($LASTEXITCODE -ne 0) {
    throw "Docker image build failed with exit code $LASTEXITCODE"
}

$mounts = @(
    "type=bind,source=$repoRoot,target=/source,readonly",
    "type=bind,source=$artifactRoot,target=/validation/evidence",
    "type=volume,source=$cacheName,target=/validation/cache"
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
    $imageName,
    'bash', '-c',
    'cp /source/scripts/linux-validation-container.sh /validation/driver.sh && exec bash /validation/driver.sh'
)

& docker @dockerArgs
if ($LASTEXITCODE -ne 0) {
    throw "Linux validation failed with exit code $LASTEXITCODE; see $artifactRoot"
}

Write-Host "Linux validation completed. Evidence: $artifactRoot"
