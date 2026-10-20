$ErrorActionPreference = "Stop"
if ($env:GITHUB_ACTIONS -ne "true") { throw "Only run on disposable CI runners" }
$installers = @(Get-ChildItem src-tauri/target/release/bundle/nsis/*.exe)
if ($installers.Count -ne 1) { throw "Expected one NSIS installer" }
$installer = $installers[0]
if ($installer.Length -lt 100000) { throw "Installer is unexpectedly small" }
$signature = Get-AuthenticodeSignature $installer.FullName
if ($signature.Status -notin @("NotSigned", "Valid")) {
    throw "Unexpected installer signature failure: $($signature.Status)"
}
$installDir = Join-Path $env:RUNNER_TEMP "VYSTRIQ-installed"
if (Test-Path $installDir) { throw "Test destination already exists" }
$database = Join-Path $env:APPDATA "com.vystriq.desktop/vystriq.sqlite"
if (Test-Path $database) { throw "Refusing to use existing app data" }
$setup = Start-Process $installer.FullName -ArgumentList "/S /D=$installDir" -PassThru
if (!$setup.WaitForExit(120000)) { throw "Installer timed out" }
if ($setup.ExitCode -ne 0) { throw "NSIS failed: $($setup.ExitCode)" }
$executable = Join-Path $installDir "vystriq.exe"
if (!(Test-Path $executable)) { throw "Installed executable missing" }
if ((Get-FileHash $executable).Hash -ne (Get-FileHash "src-tauri/target/release/vystriq.exe").Hash) {
    throw "Installed executable differs from built executable"
}
$config = Get-Content src-tauri/tauri.conf.json -Raw | ConvertFrom-Json
foreach ($entry in $config.bundle.resources.PSObject.Properties) {
    $source = Join-Path "src-tauri" $entry.Name
    $destination = Join-Path $installDir $entry.Value
    if ((Get-FileHash $source).Hash -ne (Get-FileHash $destination).Hash) {
        throw "Installed resource mismatch: $($entry.Value)"
    }
}
$app = Start-Process $executable -PassThru
try {
    Start-Sleep -Seconds 15
    if ($app.HasExited) { throw "Installed app exited: $($app.ExitCode)" }
    if (!(Test-Path $database)) { throw "Installed app did not initialize its database" }
    Write-Output "Installed application remained running and initialized its database."
} finally {
    if (!$app.HasExited) {
        Stop-Process -Id $app.Id
        $app.WaitForExit(10000) | Out-Null
    }
}
python -c "import sqlite3,sys; c=sqlite3.connect(sys.argv[1]); assert c.execute('pragma integrity_check').fetchone()[0]=='ok'; assert c.execute('pragma user_version').fetchone()[0]==4; assert c.execute('select count(*) from tasks').fetchone()[0]==0; assert c.execute('select count(*) from sessions').fetchone()[0]==0" $database
if ($LASTEXITCODE -ne 0) { throw "Database validation failed" }
$uninstallers = @(Get-ChildItem $installDir -Filter "*uninstall*.exe")
if ($uninstallers.Count -ne 1) { throw "Expected one uninstaller" }
$uninstall = Start-Process $uninstallers[0].FullName -ArgumentList "/S" -PassThru
if (!$uninstall.WaitForExit(120000)) { throw "Uninstaller timed out" }
if ($uninstall.ExitCode -ne 0) { throw "Uninstaller failed" }
for ($i = 0; $i -lt 30 -and (Test-Path $executable); $i++) { Start-Sleep -Seconds 1 }
if (Test-Path $executable) { throw "Uninstaller left the application executable" }
if (!(Test-Path $database)) { throw "Uninstaller unexpectedly removed user data" }
Write-Output "Unsigned NSIS install, resource integrity, process startup, and uninstall passed."
Write-Output "SmartScreen and interactive browser/permission flows were not exercised by this test."
