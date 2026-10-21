# Package the Tauri Windows build as an MSIX for Microsoft Store submission.
#
# This script only produces the package. It does not sign it, upload it, or
# publish anything. Microsoft re-signs Store submissions, so no certificate is
# required here; see docs/STORE_SUBMISSION.md.
#
# Example (run on Windows, after `npm ci`):
#   powershell -ExecutionPolicy Bypass -File scripts/package-msix.ps1 `
#     -IdentityName "12345Publisher.VYSTRIQ" `
#     -Publisher "CN=1A2B3C4D-0000-0000-0000-000000000000" `
#     -PublisherDisplayName "Prashant" -Build
#
# Without Partner Center values the script refuses to build a package unless
# -AllowPlaceholderIdentity is passed, which marks the output "validation" and
# must never be uploaded to the Store.

[CmdletBinding()]
param(
    [string]$Version = "",
    [string]$PackageVersion = "",
    [ValidateSet("x64", "arm64")]
    [string]$Architecture = "x64",
    [string]$IdentityName = "",
    [string]$Publisher = "",
    [string]$PublisherDisplayName = "",
    [string]$DisplayName = "VYSTRIQ",
    [string]$Description = "Local-first focus sessions, tasks and browser focus rules.",
    [string]$BuiltExecutable = "",
    [string]$OutputDirectory = "",
    [switch]$Build,
    [switch]$AllowPlaceholderIdentity
)

$ErrorActionPreference = "Stop"
$repo = Split-Path $PSScriptRoot -Parent
$configPath = Join-Path $repo "src-tauri/tauri.conf.json"
if (!(Test-Path $configPath)) { throw "Cannot find src-tauri/tauri.conf.json" }
$config = Get-Content $configPath -Raw | ConvertFrom-Json

# --- Version -------------------------------------------------------------
if (!$Version) { $Version = $config.version }
if ($Version -notmatch '^\d+\.\d+\.\d+$') {
    throw "App version '$Version' must be three segments (for example 0.4.1)."
}
$segments = @($Version.Split('.') | ForEach-Object { [int]$_ })
if (($segments | Where-Object { $_ -lt 0 -or $_ -gt 65535 })) {
    throw "App version segments must be between 0 and 65535."
}

# The Store requires a four-part package version whose first segment is not 0,
# with the fourth segment left at 0. A pre-1.0 app version therefore cannot be
# used verbatim: 0.4.1 becomes 1.4.1.0. The package version is what customers
# receive as an update, so it must always increase between submissions; use
# -PackageVersion to control it explicitly (for example 2.0.0 after 1.9.x).
if (!$PackageVersion) {
    $major = if ($segments[0] -ge 1) { $segments[0] } else { 1 }
    $PackageVersion = "$major.$($segments[1]).$($segments[2])"
}
if ($PackageVersion -notmatch '^\d+\.\d+\.\d+$') {
    throw "PackageVersion '$PackageVersion' must be three segments; the fourth MSIX segment is always 0."
}
$packageSegments = @($PackageVersion.Split('.') | ForEach-Object { [int]$_ })
if ($packageSegments[0] -lt 1 -or ($packageSegments | Where-Object { $_ -gt 65535 })) {
    throw "MSIX version segments must be between 1 and 65535, and the first cannot be 0."
}
$packageVersion = "$PackageVersion.0"
if ($Version -ne $config.version) {
    Write-Output "App version override: $Version (repository version $($config.version))"
}
Write-Output "App version $Version -> MSIX package version $packageVersion"

# --- Identity ------------------------------------------------------------
# windows/msix/identity.json holds the Partner Center values so a local build and
# CI cannot drift from the reserved product. Explicit parameters still win.
$identityMetadata = $null
$identityPath = Join-Path $repo "windows/msix/identity.json"
if (Test-Path $identityPath) {
    $identityMetadata = Get-Content $identityPath -Raw | ConvertFrom-Json
    if ($IdentityName -and $IdentityName -ne $identityMetadata.identityName) {
        Write-Warning "-IdentityName overrides the value in windows/msix/identity.json."
    }
    if ($Publisher -and $Publisher -ne $identityMetadata.publisher) {
        Write-Warning "-Publisher overrides the value in windows/msix/identity.json."
    }
    if (!$IdentityName) { $IdentityName = $identityMetadata.identityName }
    if (!$Publisher) { $Publisher = $identityMetadata.publisher }
    if (!$PublisherDisplayName) { $PublisherDisplayName = $identityMetadata.publisherDisplayName }
}
$placeholder = $false
if (!$IdentityName -or !$Publisher -or !$PublisherDisplayName) {
    if (!$AllowPlaceholderIdentity) {
        throw @"
Partner Center identity is required. Reserve the product name in Partner Center
(Product management > App identity) and pass the exact values:

  -IdentityName         (for example 12345Publisher.VYSTRIQ)
  -Publisher            (CN=... GUID assigned by the Store)
  -PublisherDisplayName (the name shown to customers)

See docs/STORE_SUBMISSION.md. For a local, non-submittable validation build add
-AllowPlaceholderIdentity.
"@
    }
    $placeholder = $true
    if (!$IdentityName) { $IdentityName = "VYSTRIQ.Validation" }
    if (!$Publisher) { $Publisher = "CN=VYSTRIQ Validation Build" }
    if (!$PublisherDisplayName) { $PublisherDisplayName = "VYSTRIQ validation build" }
}
if ($Publisher -notmatch '^CN=') {
    throw "Publisher must be the exact distinguished name from Partner Center and start with CN= (a bare name is rejected by the Store)."
}
if ($IdentityName -notmatch '^[A-Za-z0-9.\-]+$') {
    throw "IdentityName may only contain letters, digits, periods and dashes."
}

# --- Build output --------------------------------------------------------
if ($Build) {
    Push-Location $repo
    try {
        npm run desktop:build -- --no-bundle
        if ($LASTEXITCODE -ne 0) { throw "Tauri build failed ($LASTEXITCODE)" }
    } finally {
        Pop-Location
    }
}
if (!$BuiltExecutable) {
    $targetDir = if ($Architecture -eq "arm64") { "aarch64-pc-windows-msvc" } else { "x86_64-pc-windows-msvc" }
    $candidates = @(
        (Join-Path $repo "src-tauri/target/$targetDir/release/vystriq.exe"),
        (Join-Path $repo "src-tauri/target/release/vystriq.exe")
    )
    $BuiltExecutable = $candidates | Where-Object { Test-Path $_ } | Select-Object -First 1
}
if (!$BuiltExecutable -or !(Test-Path $BuiltExecutable)) {
    throw "Build output vystriq.exe was not found. Build the Windows app first (npm run desktop:build -- --no-bundle) or pass -BuiltExecutable."
}
$BuiltExecutable = (Resolve-Path $BuiltExecutable).Path

# --- Staging layout ------------------------------------------------------
if (!$OutputDirectory) { $OutputDirectory = Join-Path $repo "src-tauri/target/msix" }
$staging = Join-Path $repo "src-tauri/target/msix/stage/$Architecture"
$assets = Join-Path $staging "Assets"
if (Test-Path $staging) { Remove-Item $staging -Recurse -Force }
New-Item -ItemType Directory -Path $assets -Force | Out-Null

Copy-Item $BuiltExecutable (Join-Path $staging "VYSTRIQ.exe")

# Bundled web resources, at the same relative paths used by the NSIS build.
foreach ($entry in $config.bundle.resources.PSObject.Properties) {
    $source = Join-Path (Join-Path $repo "src-tauri") $entry.Name
    $destination = Join-Path $staging $entry.Value
    if (!(Test-Path $source)) { throw "Missing bundled resource: $($entry.Name)" }
    New-Item -ItemType Directory -Path (Split-Path $destination -Parent) -Force | Out-Null
    Copy-Item $source $destination
}

# Store visual assets. Sizes are validated by scripts/verify-msix.ps1.
$assetNames = @(
    "StoreLogo.png",
    "Square44x44Logo.png",
    "Square71x71Logo.png",
    "Square150x150Logo.png",
    "Square310x310Logo.png",
    "Wide310x150Logo.png"
)
foreach ($name in $assetNames) {
    $source = Join-Path $repo "src-tauri/icons/$name"
    if (!(Test-Path $source)) { throw "Missing Store asset: src-tauri/icons/$name" }
    Copy-Item $source (Join-Path $assets $name)
}

# --- Manifest ------------------------------------------------------------
$escape = { param($value) [System.Security.SecurityElement]::Escape($value) }
$manifest = Get-Content (Join-Path $repo "windows/msix/AppxManifest.xml.template") -Raw
$tokens = @{
    "__IDENTITY_NAME__"        = & $escape $IdentityName
    "__PUBLISHER__"            = & $escape $Publisher
    "__PUBLISHER_DISPLAY_NAME__" = & $escape $PublisherDisplayName
    "__VERSION__"              = $packageVersion
    "__ARCHITECTURE__"         = $Architecture
    "__DISPLAY_NAME__"         = & $escape $DisplayName
    "__DESCRIPTION__"          = & $escape $Description
}
foreach ($token in $tokens.Keys) { $manifest = $manifest.Replace($token, $tokens[$token]) }
$unreplaced = @($tokens.Keys | Where-Object { $manifest.Contains($_) })
if ($unreplaced.Count -gt 0) { throw "Unreplaced manifest tokens: $($unreplaced -join ', ')" }
# Write UTF-8 without a BOM so the XML declaration stays the first bytes.
[System.IO.File]::WriteAllText(
    (Join-Path $staging "AppxManifest.xml"),
    $manifest,
    (New-Object System.Text.UTF8Encoding($false))
)

# --- Pack ----------------------------------------------------------------
function Find-MakeAppx {
    $command = Get-Command makeappx.exe -ErrorAction SilentlyContinue
    if ($command) { return $command.Source }
    $kits = "${env:ProgramFiles(x86)}\Windows Kits\10\bin"
    if (Test-Path $kits) {
        $candidate = Get-ChildItem $kits -Recurse -Filter makeappx.exe -ErrorAction SilentlyContinue |
            Where-Object { $_.FullName -match '\\x64\\makeappx\.exe$' } |
            Sort-Object FullName -Descending | Select-Object -First 1
        if ($candidate) { return $candidate.FullName }
    }
    throw "makeappx.exe was not found. Install the Windows SDK (the windows-2022 runner includes it)."
}

$makeappx = Find-MakeAppx
New-Item -ItemType Directory -Path $OutputDirectory -Force | Out-Null
$suffix = if ($placeholder) { "-validation" } else { "" }
$packagePath = Join-Path $OutputDirectory "VYSTRIQ_${Version}_${Architecture}${suffix}.msix"
if (Test-Path $packagePath) { Remove-Item $packagePath -Force }
& $makeappx pack /o /d $staging /p $packagePath
if ($LASTEXITCODE -ne 0) { throw "makeappx pack failed ($LASTEXITCODE)" }

# --- Verify and record ---------------------------------------------------
& (Join-Path $PSScriptRoot "verify-msix.ps1") -Package $packagePath `
    -ExpectedIdentity $IdentityName -ExpectedPublisher $Publisher `
    -ExpectedVersion $packageVersion -ExpectedArchitecture $Architecture
if ($LASTEXITCODE -ne 0) { throw "MSIX verification failed" }

$packageHash = (Get-FileHash $packagePath -Algorithm SHA256).Hash.ToLower()
$exeHash = (Get-FileHash $BuiltExecutable -Algorithm SHA256).Hash.ToLower()
$commit = ""
Push-Location $repo
try { $commit = (git rev-parse HEAD 2>$null) } catch { $commit = "unknown" }
Pop-Location

$info = [ordered]@{
    package               = Split-Path $packagePath -Leaf
    sha256                = $packageHash
    app_version           = $Version
    package_version       = $packageVersion
    architecture          = $Architecture
    identity_name         = $IdentityName
    publisher             = $Publisher
    publisher_display_name = $PublisherDisplayName
    placeholder_identity  = $placeholder
    source_executable     = $BuiltExecutable
    source_executable_sha256 = $exeHash
    commit                = $commit
    store_id              = if ($identityMetadata) { $identityMetadata.storeId } else { $null }
    package_family_name   = if ($identityMetadata) { $identityMetadata.packageFamilyName } else { $null }
    makeappx              = $makeappx
    signed                = $false
    note                  = if ($placeholder) { "Validation only. Do not upload: Partner Center identity is missing." } else { "Unsigned. The Microsoft Store re-signs accepted submissions." }
}
$infoPath = Join-Path $OutputDirectory "msix-build-info.json"
$info | ConvertTo-Json -Depth 4 | Set-Content -Path $infoPath -Encoding UTF8
"$packageHash  $(Split-Path $packagePath -Leaf)" | Set-Content -Path (Join-Path $OutputDirectory "SHA256SUMS.txt") -Encoding ASCII

Write-Output ""
Write-Output "Package:  $packagePath"
Write-Output "SHA-256:  $packageHash"
Write-Output "Info:     $infoPath"
if ($placeholder) {
    Write-Warning "Placeholder identity used. This package cannot be submitted to the Microsoft Store and will not install as a Store product."
}
Write-Output "Not done here: signing, installation, launch/update/uninstall testing, Partner Center upload."
