# Validate an MSIX package produced by scripts/package-msix.ps1.
#
# Checks the unpacked contents, manifest identity and capabilities, Store
# visual asset sizes, bundled extension files, and that no user data or secrets
# were packaged. It does not install the package: installation, launch and
# update behaviour must be tested on Windows (see docs/STORE_SUBMISSION.md).
#
# Usage:
#   powershell -ExecutionPolicy Bypass -File scripts/verify-msix.ps1 `
#     -Package src-tauri/target/msix/VYSTRIQ_0.4.1_x64.msix `
#     -ExpectedIdentity 12345Publisher.VYSTRIQ `
#     -ExpectedPublisher "CN=1A2B3C4D-0000-0000-0000-000000000000" `
#     -ExpectedVersion 0.4.1.0 -ExpectedArchitecture x64

[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$Package,
    [string]$ExpectedIdentity = "",
    [string]$ExpectedPublisher = "",
    [string]$ExpectedVersion = "",
    [ValidateSet("x64", "arm64", "")]
    [string]$ExpectedArchitecture = "",
    [switch]$KeepExtracted
)

$ErrorActionPreference = "Stop"
$repo = Split-Path $PSScriptRoot -Parent
$failures = New-Object System.Collections.Generic.List[string]
function Check([bool]$condition, [string]$message) {
    if ($condition) {
        Write-Output "  ok   $message"
    } else {
        Write-Output "  FAIL $message"
        $failures.Add($message)
    }
}

function Get-PngSize([string]$path) {
    $bytes = [System.IO.File]::ReadAllBytes($path)
    if ($bytes.Length -lt 24) { return $null }
    $signature = [BitConverter]::ToString($bytes[0..7])
    if ($signature -ne "89-50-4E-47-0D-0A-1A-0A") { return $null }
    # PNG IHDR stores width and height as big-endian 32-bit integers.
    $width = ([uint32]$bytes[16] -shl 24) -bor ([uint32]$bytes[17] -shl 16) -bor ([uint32]$bytes[18] -shl 8) -bor [uint32]$bytes[19]
    $height = ([uint32]$bytes[20] -shl 24) -bor ([uint32]$bytes[21] -shl 16) -bor ([uint32]$bytes[22] -shl 8) -bor [uint32]$bytes[23]
    return @{ Width = $width; Height = $height }
}

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
    throw "makeappx.exe was not found. Install the Windows SDK."
}

if (!(Test-Path $Package)) { throw "Package not found: $Package" }
$Package = (Resolve-Path $Package).Path
Write-Output "Verifying $Package"
$packageHash = (Get-FileHash $Package -Algorithm SHA256).Hash.ToLower()
Write-Output "  sha256 $packageHash"

$makeappx = Find-MakeAppx
$extract = Join-Path ([System.IO.Path]::GetTempPath()) ("vystriq-msix-" + [guid]::NewGuid().ToString("N"))
try {
    & $makeappx unpack /o /p $Package /d $extract | Out-Null
    if ($LASTEXITCODE -ne 0) { throw "makeappx unpack failed ($LASTEXITCODE); the file is not a readable MSIX" }

    Check (Test-Path (Join-Path $extract "AppxManifest.xml")) "AppxManifest.xml is present"
    Check (Test-Path (Join-Path $extract "AppxBlockMap.xml")) "AppxBlockMap.xml is present"

    $manifestPath = Join-Path $extract "AppxManifest.xml"
    [xml]$manifest = Get-Content $manifestPath -Raw
    $identity = $manifest.Package.Identity
    $architecture = $manifest.Package.Identity.ProcessorArchitecture

    if ($ExpectedIdentity) { Check ($identity.Name -ceq $ExpectedIdentity) "Identity Name is '$ExpectedIdentity'" }
    if ($ExpectedPublisher) { Check ($identity.Publisher -ceq $ExpectedPublisher) "Identity Publisher matches Partner Center" }
    if ($ExpectedVersion) { Check ($identity.Version -ceq $ExpectedVersion) "Identity Version is '$ExpectedVersion'" }
    if ($ExpectedArchitecture) { Check ($architecture -ceq $ExpectedArchitecture) "ProcessorArchitecture is '$ExpectedArchitecture'" }
    Check ($identity.Version -match '^\d+\.\d+\.\d+\.0$') "Version leaves the fourth Store-reserved segment at 0"
    if ($ExpectedIdentity -and $ExpectedIdentity -ne "VYSTRIQ.Validation") {
        Check ($identity.Name -notmatch 'Validation') "Identity is not the validation placeholder"
        Check ($identity.Publisher -notmatch 'Validation Build') "Publisher is not the validation placeholder"
    }

    $applications = @($manifest.Package.Applications.Application)
    Check ($applications.Count -eq 1) "Exactly one Application entry"
    $application = $applications[0]
    Check ($application.EntryPoint -eq "Windows.FullTrustApplication") "Entry point is Windows.FullTrustApplication"
    $executable = Join-Path $extract $application.Executable
    Check (Test-Path $executable) "Declared executable $($application.Executable) exists"

    $capabilities = @($manifest.Package.Capabilities.ChildNodes | ForEach-Object { $_.Name })
    Check ($capabilities -contains "runFullTrust") "runFullTrust restricted capability is declared"

    $families = @($manifest.Package.Dependencies.TargetDeviceFamily)
    Check ($families.Count -eq 1 -and $families[0].Name -eq "Windows.Desktop") "Targets Windows.Desktop"
    Check ([version]$families[0].MinVersion -ge [version]"10.0.17763.0") "Minimum OS is Windows 10 1809 or newer (WebView2 requirement)"

    $startup = @($manifest.SelectNodes("//*[local-name()='StartupTask']"))
    Check ($startup.Count -eq 1) "Exactly one startupTask is declared"
    if ($startup.Count -eq 1) {
        Check ($startup[0].TaskId -eq "VYSTRIQ") "startupTask TaskId matches the app's startup task id"
        Check ($startup[0].Enabled -eq "false") "startupTask is disabled by default (opt-in only)"
    }

    # Visual assets required by the manifest and by Partner Center.
    $sizes = @{
        "Assets/StoreLogo.png"        = @(50, 50)
        "Assets/Square44x44Logo.png"  = @(44, 44)
        "Assets/Square71x71Logo.png"  = @(71, 71)
        "Assets/Square150x150Logo.png" = @(150, 150)
        "Assets/Square310x310Logo.png" = @(310, 310)
    }
    foreach ($asset in $sizes.Keys) {
        $path = Join-Path $extract $asset
        $exists = Test-Path $path
        Check $exists "$asset is present"
        if ($exists) {
            $size = Get-PngSize $path
            $expected = $sizes[$asset]
            Check ($null -ne $size -and $size.Width -eq $expected[0] -and $size.Height -eq $expected[1]) "$asset is $($expected[0])x$($expected[1])"
        }
    }

    # Bundled browser extension resources must match the repository byte for byte.
    $config = Get-Content (Join-Path $repo "src-tauri/tauri.conf.json") -Raw | ConvertFrom-Json
    foreach ($entry in $config.bundle.resources.PSObject.Properties) {
        $source = Join-Path (Join-Path $repo "src-tauri") $entry.Name
        $packaged = Join-Path $extract $entry.Value
        if (!(Test-Path $source)) { continue }
        Check ((Test-Path $packaged) -and ((Get-FileHash $source).Hash -eq (Get-FileHash $packaged).Hash)) "resource $($entry.Value) matches the repository"
    }

    # Never ship user data, pairing secrets, symbols or signing material.
    $forbidden = @(Get-ChildItem $extract -Recurse -File | Where-Object {
        $_.Name -match '\.(sqlite|sqlite3|db|pdb|pfx|cer|key|pem)$' -or
        $_.Name -eq "browser-token" -or $_.Name -eq "lifecycle.json"
    })
    Check ($forbidden.Count -eq 0) "No database, pairing token, symbol or signing files are packaged"
    if ($forbidden.Count -gt 0) { $forbidden | ForEach-Object { Write-Output "       packaged: $($_.FullName.Substring($extract.Length + 1))" } }

    $total = (Get-ChildItem $extract -Recurse -File | Measure-Object Length -Sum).Sum
    Write-Output ("  info uncompressed size: {0:N0} bytes" -f $total)
    Write-Output ("  info package size:      {0:N0} bytes" -f (Get-Item $Package).Length)
} finally {
    if (!$KeepExtracted -and (Test-Path $extract)) { Remove-Item $extract -Recurse -Force }
    elseif ($KeepExtracted) { Write-Output "  extracted to $extract" }
}

if ($failures.Count -gt 0) {
    Write-Output ""
    Write-Output "FAILED: $($failures.Count) check(s) did not pass."
    exit 1
}
Write-Output ""
Write-Output "All package structure checks passed."
Write-Output "Not verified here: installation, launch, update, uninstall, runtime permissions, camera, browser pairing."
exit 0
