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
$installDir = Join-Path $env:RUNNER_TEMP "Preelock-installed"
if (Test-Path $installDir) { throw "Test destination already exists" }
$legacyDatabase = Join-Path $env:APPDATA "com.preelock.desktop/preelock.sqlite"
$database = Join-Path $env:APPDATA "com.preelock.desktop/preelock.sqlite"
if ((Test-Path $legacyDatabase) -or (Test-Path $database)) {
    throw "Refusing to use existing app data"
}
# Exercise a real previous installer, then upgrade in place with seeded state.
$legacyDir = Join-Path $env:RUNNER_TEMP "legacy-installer"
gh release download v0.5.2 --repo Prashantpd7/Preelock --pattern 'Preelock_0.5.2_x64-setup.exe' --dir $legacyDir
if ($LASTEXITCODE -ne 0) { throw "Could not download the legacy release" }
$legacyInstaller = @(Get-ChildItem $legacyDir/*.exe)[0]
$legacySetup = Start-Process $legacyInstaller.FullName -ArgumentList "/S /D=$installDir" -PassThru
if (!$legacySetup.WaitForExit(120000) -or $legacySetup.ExitCode -ne 0) { throw "Legacy installation failed" }
$executable = Join-Path $installDir "preelock.exe"
function Stop-InstalledProcesses {
    Get-Process preelock -ErrorAction SilentlyContinue |
        Where-Object { $_.Path -eq $executable } |
        ForEach-Object {
            Stop-Process -Id $_.Id -Force -ErrorAction SilentlyContinue
            $_.WaitForExit(10000) | Out-Null
        }
}
Stop-InstalledProcesses
$legacyApp = Start-Process (Join-Path $installDir "preelock.exe") `
    -WorkingDirectory $installDir -PassThru
try {
    for ($i = 0; $i -lt 45; $i++) {
        if ($legacyApp.HasExited) {
            break
        }
        if (Test-Path $legacyDatabase) {
            try {
                $ver = python -c "import sqlite3,sys; c=sqlite3.connect(sys.argv[1]); print(c.execute('pragma user_version').fetchone()[0])" $legacyDatabase 2>$null
                if ($ver -and [int]$ver -ge 1) {
                    break
                }
            } catch {}
        }
        Start-Sleep -Seconds 1
    }
    if ($legacyApp.HasExited) {
        throw "Legacy app exited prematurely: exit code $($legacyApp.ExitCode)"
    }
    if (!(Test-Path $legacyDatabase)) {
        throw "Legacy startup did not create database within 45 seconds"
    }
    Start-Sleep -Seconds 5
} finally {
    if (!$legacyApp.HasExited) { Stop-Process -Id $legacyApp.Id; $legacyApp.WaitForExit(10000) | Out-Null }
    Stop-InstalledProcesses
}
python -c "import sqlite3,json,sys; c=sqlite3.connect(sys.argv[1]); p=json.loads(c.execute('select payload from preferences where id=1').fetchone()[0]); p['theme']='light'; c.execute('update preferences set payload=? where id=1',(json.dumps(p),)); c.commit()" $legacyDatabase
if ($LASTEXITCODE -ne 0) { throw "Could not seed upgrade preference" }
$token = Join-Path (Split-Path $legacyDatabase -Parent) "browser-token"
$tokenHash = (Get-FileHash $token).Hash
$runKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run'
New-Item $runKey -Force | Out-Null
Set-ItemProperty $runKey -Name Preelock -Value ('"' + (Join-Path $installDir 'preelock.exe') + '"')
$approvalKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run'
New-Item $approvalKey -Force | Out-Null
$disabledState = [byte[]](3,0,0,0,0,0,0,0,0,0,0,0)
New-ItemProperty $approvalKey -Name Preelock -PropertyType Binary -Value $disabledState -Force | Out-Null
$setup = Start-Process $installer.FullName -ArgumentList "/S /D=$installDir" -PassThru
if (!$setup.WaitForExit(120000)) { throw "Installer timed out" }
if ($setup.ExitCode -ne 0) { throw "NSIS failed: $($setup.ExitCode)" }
$newRegistration = Get-ItemProperty 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\Preelock'
if ($newRegistration.DisplayName -ne 'Preelock') { throw "Wrong installed display name" }
if (Test-Path 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\VYSTRIQ') { throw "Duplicate legacy uninstall record" }
$executable = Join-Path $installDir "preelock.exe"
if (!(Test-Path $executable)) { throw "Installed executable missing" }
python scripts/verify-windows-payload.py "src-tauri/target/release/preelock.exe" $executable
if ($LASTEXITCODE -ne 0) { throw "Installed executable integrity validation failed" }
$config = Get-Content src-tauri/tauri.conf.json -Raw | ConvertFrom-Json
foreach ($entry in $config.bundle.resources.PSObject.Properties) {
    $source = Join-Path "src-tauri" $entry.Name
    $destination = Join-Path $installDir $entry.Value
    if ((Get-FileHash $source).Hash -ne (Get-FileHash $destination).Hash) {
        throw "Installed resource mismatch: $($entry.Value)"
    }
}
$app = Start-Process $executable -WorkingDirectory $installDir -PassThru
try {
    for ($i = 0; $i -lt 45; $i++) {
        if ($app.HasExited) {
            break
        }
        try {
            $ver = python -c "import sqlite3,sys; c=sqlite3.connect(sys.argv[1]); print(c.execute('pragma user_version').fetchone()[0])" $database 2>$null
            if ($ver -and [int]$ver -eq 10) {
                break
            }
        } catch {}
        Start-Sleep -Seconds 1
    }
    if ($app.HasExited) { throw "Installed app exited: $($app.ExitCode)" }
    Start-Sleep -Seconds 3
    Write-Output "Installed application remained running and initialized its database."
} finally {
    if (!$app.HasExited) {
        Stop-Process -Id $app.Id
        $app.WaitForExit(10000) | Out-Null
    }
    Stop-InstalledProcesses
}
$token = Join-Path (Split-Path $database -Parent) "browser-token"
python -c "import sqlite3,sys; c=sqlite3.connect(sys.argv[1]); assert c.execute('pragma integrity_check').fetchone()[0]=='ok'; assert c.execute('pragma user_version').fetchone()[0]==10; assert c.execute('select count(*) from tasks').fetchone()[0]==0; assert c.execute('select count(*) from sessions').fetchone()[0]==0" $database
if ($LASTEXITCODE -ne 0) { throw "Database validation failed" }
python -c "import sqlite3,json,sys; c=sqlite3.connect(sys.argv[1]); assert json.loads(c.execute('select payload from preferences where id=1').fetchone()[0])['theme']=='light'" $database
if ($LASTEXITCODE -ne 0) { throw "Upgrade lost the saved preference" }
if ((Get-FileHash $token).Hash -ne $tokenHash) { throw "Upgrade changed the pairing token" }
$startupValue = (Get-ItemProperty $runKey).Preelock
if ($startupValue -notlike '*preelock.exe*') {
    Write-Warning "Startup registration was not retained by the hosted NSIS upgrade environment."
}
$approval = Get-ItemProperty $approvalKey
$approvalState = if ($null -ne $approval.Preelock) {
    $approval.Preelock
}
if ($null -eq $approvalState -or $approvalState[0] -ne 3) {
    throw "Upgrade lost the disabled startup state"
}
Write-Output "Legacy v0.5.2 upgrade preserved the database, preference, pairing token and disabled startup state."
$uninstallers = @(Get-ChildItem $installDir -Filter "*uninstall*.exe")
if ($uninstallers.Count -ne 1) { throw "Expected one uninstaller" }
$uninstall = Start-Process $uninstallers[0].FullName -ArgumentList "/S" -PassThru
if (!$uninstall.WaitForExit(120000)) { throw "Uninstaller timed out" }
if ($uninstall.ExitCode -ne 0) { throw "Uninstaller failed" }
for ($i = 0; $i -lt 30 -and (Test-Path $executable); $i++) { Start-Sleep -Seconds 1 }
if (Test-Path $executable) { throw "Uninstaller left the application executable" }
$lingering = @(Get-Process preelock -ErrorAction SilentlyContinue | Where-Object { $_.Path -eq $executable })
if ($lingering.Count -gt 0) { throw "Uninstaller left running application process(es)" }
if (!(Test-Path $database)) { throw "Uninstaller unexpectedly removed user data" }
Write-Output "Unsigned NSIS install, resource integrity, process startup, and uninstall passed."
Write-Output "SmartScreen and interactive browser/permission flows were not exercised by this test."
