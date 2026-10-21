# Sign an MSIX with a free, self-signed certificate so it can be sideloaded on
# this Windows machine for local testing.
#
# This is NOT for distribution and is NOT a Store submission step. The Store
# re-signs packages itself. A self-signed package can only be installed on a
# machine where the certificate is trusted and the certificate subject exactly
# matches the package's Publisher value, so it must be paired with a validation
# build:
#
#   powershell -File scripts/package-msix.ps1 -AllowPlaceholderIdentity `
#     -IdentityName "VYSTRIQ.LocalTest" -Publisher "CN=VYSTRIQ Local Test"
#   powershell -File scripts/sign-msix-dev.ps1 `
#     -Package src-tauri/target/msix/VYSTRIQ_0.4.1_x64-validation.msix
#
# Then, in a normal (non-elevated) PowerShell window:
#   Add-AppxPackage -Path <package>
#   Get-AppxPackage VYSTRIQ.LocalTest | Select PackageFullName, InstallLocation
#   Remove-AppxPackage -Package <PackageFullName>

[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$Package,
    [string]$Subject = "CN=VYSTRIQ Local Test",
    [switch]$TrustCertificate
)

$ErrorActionPreference = "Stop"
if (!(Test-Path $Package)) { throw "Package not found: $Package" }
$Package = (Resolve-Path $Package).Path

# The MSIX publisher must equal the signing certificate subject.
$extract = Join-Path ([System.IO.Path]::GetTempPath()) ("vystriq-sign-" + [guid]::NewGuid().ToString("N"))
New-Item -ItemType Directory -Path $extract -Force | Out-Null
try {
    Add-Type -AssemblyName System.IO.Compression.FileSystem
    [System.IO.Compression.ZipFile]::ExtractToDirectory($Package, $extract)
    [xml]$manifest = Get-Content (Join-Path $extract "AppxManifest.xml") -Raw
    $publisher = $manifest.Package.Identity.Publisher
} finally {
    Remove-Item $extract -Recurse -Force
}
if ($publisher -ne $Subject) {
    throw "Package Publisher '$publisher' does not match the certificate subject '$Subject'. Rebuild the validation package with -Publisher `"$Subject`" or sign with a certificate whose subject matches Partner Center."
}

$signtool = Get-Command signtool.exe -ErrorAction SilentlyContinue
if (!$signtool) {
    $kits = "${env:ProgramFiles(x86)}\Windows Kits\10\bin"
    $signtool = Get-ChildItem $kits -Recurse -Filter signtool.exe -ErrorAction SilentlyContinue |
        Where-Object { $_.FullName -match '\\x64\\signtool\.exe$' } |
        Sort-Object FullName -Descending | Select-Object -First 1
}
if (!$signtool) { throw "signtool.exe was not found. Install the Windows SDK." }

$certificate = Get-ChildItem Cert:\CurrentUser\My |
    Where-Object { $_.Subject -eq $Subject -and $_.HasPrivateKey } |
    Sort-Object NotAfter -Descending | Select-Object -First 1
if (!$certificate) {
    Write-Output "Creating a self-signed code signing certificate for '$Subject'."
    $certificate = New-SelfSignedCertificate `
        -Type Custom `
        -Subject $Subject `
        -FriendlyName "VYSTRIQ local test signing" `
        -KeyUsage DigitalSignature `
        -KeyAlgorithm RSA `
        -KeyLength 2048 `
        -CertStoreLocation "Cert:\CurrentUser\My" `
        -TextExtension @("2.5.29.37={text}1.3.6.1.5.5.7.3.3", "2.5.29.19={text}")
}
$cerPath = Join-Path (Split-Path $Package -Parent) "vystriq-local-test.cer"
Export-Certificate -Cert $certificate -FilePath $cerPath -Force | Out-Null

if ($TrustCertificate) {
    Import-Certificate -FilePath $cerPath -CertStoreLocation "Cert:\CurrentUser\TrustedPeople" | Out-Null
    Write-Output "Trusted the local test certificate in CurrentUser\TrustedPeople."
} else {
    Write-Output "Certificate exported to $cerPath (not trusted yet; re-run with -TrustCertificate)."
}

& $signtool.Source sign /fd SHA256 /sha1 $certificate.Thumbprint /s My $Package
if ($LASTEXITCODE -ne 0) { throw "signtool failed ($LASTEXITCODE)" }

$result = Get-AuthenticodeSignature $Package
Write-Output "Signature status: $($result.Status) ($($result.SignerCertificate.Subject))"
Write-Warning "Self-signed package: local testing only. Never upload it to the Store and do not publish it."
