# Install Preelock — free public beta

Download only from [Prashantpd7/Preelock releases](https://github.com/Prashantpd7/Preelock/releases).
Choose the newest **Preelock 0.5.0 or later** beta. Do not use the old v0.4.0 Mac installer:
its application bundle signature is defective.

Preelock is free and needs no account, subscription, programming tools, Node.js,
or Rust. The Mac app is ad-hoc signed, **not Apple-notarized**. The Windows app
is **unsigned**. These builds are not certified or officially trusted by either
OS vendor. Download checksums establish file integrity, not publisher trust.
Some computers prohibit installation; manual approval is not always available.

## Which download?

| Computer                                   | File ending      | Requirements                |
| ------------------------------------------ | ---------------- | --------------------------- |
| Mac with Apple Silicon (M1 and newer)      | `_aarch64.dmg`   | macOS 12 or newer           |
| Intel Mac                                  | `_x64.dmg`       | macOS 12 or newer           |
| Windows PC with Intel/AMD 64-bit processor | `_x64-setup.exe` | Windows 10/11 x64; WebView2 |

On Mac, choose Apple menu → About This Mac. “Chip: Apple M…” means Apple Silicon;
“Processor: Intel…” means Intel. Windows ARM and 32-bit builds are not provided.
Internet is needed for the download and, on Windows, if the installer needs to
install Microsoft's WebView2 runtime. Older minimum OS versions are compatibility
targets, not a promise of testing on every OS version.

## Mac installation

1. Open the correct downloaded DMG in Finder.
2. Drag **Preelock** onto **Applications**. This is the normal Mac installer;
   there is no separate installation wizard. Wait for the copy to finish.
3. Eject the Preelock disk image using Finder's eject button.
4. Open Finder → Applications → **Preelock**.
5. If macOS says the developer cannot be verified or Apple cannot check the app,
   and you trust this download, open **System Settings → Privacy & Security**.
   Find the blocked Preelock message and choose **Open Anyway**, if offered.
   Authenticate and confirm **Open** when asked. On macOS 12, use
   **System Preferences → Security & Privacy → General** instead.
6. If that option is missing or your Mac is managed, stop and ask your administrator.
   Do not disable Gatekeeper or change system-wide protection settings.

Apple explains this approval flow in
[Safely open apps on your Mac](https://support.apple.com/102445).
Approval applies to an app you choose to trust; it does not notarize the app.

### “Damaged,” “will damage your computer,” or installation still blocked

Do not treat every warning as harmless. Do not override an explicit malware
warning. If “damaged” appears, confirm you have the new release and the correct
architecture, then download a fresh copy from the release page. If it persists,
report the exact message, macOS version, and installer filename through
[GitHub Issues](https://github.com/Prashantpd7/Preelock/issues).
A maintainer should compare the published checksum and inspect the bundle signature.
If integrity is correct but the OS still refuses the app, that machine's policy
may prevent this free build. Do not remove quarantine metadata or run security
bypass commands. Repeated downloads do not repair the defective v0.4.0 signature.

If the disk image cannot be ejected, close its Finder window and quit any app
running from it, then try Finder's eject button again. Launch the installed copy
from Applications, not the icon inside the DMG.

## Windows installation

1. Download the file ending in **`_x64-setup.exe`** and open it.
2. SmartScreen may say **Windows protected your PC** because the installer has
   no trusted publisher signature/reputation. If you trust this exact download,
   choose **More info → Run anyway**, only if offered.
3. Follow the installer. Preelock installs for your current Windows account;
   the app does not need developer tools or an administrator service.
   Let Microsoft's WebView2 setup finish if it is needed.
4. Open **Preelock** from Start.
5. Follow the app's **Permissions & setup** guide.

If Smart App Control, antivirus, or your organization's policy blocks it without
an approval option, stop and contact your administrator. Do not disable Defender,
SmartScreen, antivirus, or Smart App Control. Microsoft documents
[SmartScreen reputation](https://learn.microsoft.com/windows/apps/package-and-deploy/smartscreen-reputation)
and [Smart App Control](https://support.microsoft.com/en-us/windows/security/threat-malware-protection/smart-app-control-frequently-asked-questions).

A blank window or immediate close may indicate WebView2 is missing or blocked.
Try the installer again while online; consult your administrator if runtime
installation is blocked. Report the installer version, Windows version, and error.

## Permissions and first run

Open **Settings → Connections → Permissions & setup** to revisit the guide.

- **Camera (Mac only, optional):** choose **Allow camera** and respond to the Mac
  prompt. If previously denied, use Privacy & Security → Camera → Preelock, then
  return to the app. Camera presence is local; no frames are saved or uploaded.
  If unavailable, choose **Apps only**. Camera presence is not implemented on Windows.
- **Activity/app control:** grant consent within each session. Preelock observes
  the foreground app and idle time, and can hide/minimize selected distracting
  apps. The current implementation does not request Accessibility, Screen Recording,
  or Input Monitoring permission and does not record screens or keystrokes.
- **Notifications (optional):** approve the prompt or enable Preelock in the OS's
  Notifications settings. Do Not Disturb may suppress notifications.
- **Browser blocking:** requires the extension below. Preelock communicates with
  it through a local loopback connection. It does not install firewall, DNS, VPN,
  system-wide network policies, privileged services, or managed restrictions.

Your tasks and settings are stored locally. Reinstalling/upgrading normally keeps
existing data; installing under a new OS user starts with empty data.

## Browser extension setup

No extension-store listing is available yet. This step is manual and uses the
browser's extension Developer mode; no programming tools are needed.

1. Keep Preelock and your supported Chromium browser open (Chrome, Edge, Brave,
   Vivaldi, or Comet). In Preelock's Connections section, select your browser and
   open its setup guide.
2. Choose **Open extensions page**, then enable **Developer mode** there.
3. Choose **Load unpacked** and select the extension folder shown by Preelock.
   Use **Copy folder path**: on Mac press **Command–Shift–G** in the folder picker,
   paste the path, and select the folder. On Windows paste it into the folder
   picker's address bar, press Enter, and select the folder.
4. Copy the **connection code** from Preelock. Open the Preelock browser extension,
   paste the code, select the correct browser, and choose **Connect**.
   Do not share this private connection code.
5. Return to Preelock, refresh/check the connection, and use **Test blocking**.
   Pair the browser profile you actually use. Repeat setup for other supported
   browsers you want to use with Preelock.
6. To filter private windows, enable **Allow in Incognito** (or the browser's
   equivalent) in the extension's details, where supported.
7. After updating Preelock, reload the extension on the browser's extensions page.

If your browser blocks unpacked extensions, website filtering is unavailable
under that policy. Safari/Firefox are not supported. Core task features and
suitable desktop-only sessions do not require the extension. Blocking is scoped
to paired browser profiles and is not impossible to bypass.

## Uninstall safely

Wait for the active focus session to finish, remove the Preelock browser
extension, and quit Preelock from its menu-bar/system-tray menu.
On Mac, move Preelock from Applications to Trash. On Windows, open
Settings → Apps → Installed apps (Apps & features on Windows 10), find Preelock,
and choose **Uninstall**.

Uninstalling preserves local history/settings for a later reinstall. Back up in
Preelock before separately removing any personal data. Do not delete the source
repository or unrelated application folders to uninstall the app.

## What has been tested?

Read the release notes. A successful build is not a complete real-device test.
Automated runner startup does not exercise browser-downloaded Gatekeeper or
SmartScreen approval, camera consent, browser pairing, or every focus behavior.
Those flows need manual testing on your own supported devices. This is a public
beta; report failures with the OS version and exact message.

## Updating an existing installation

Quit the old application before installing. On Mac, install Preelock.app and
verify your existing data before removing the old application bundle. Do not
run both applications together or remove the application-data directory.
Windows direct installers reuse the previous installation location when detected.
Store packages retain the existing identity; Store update testing and certification
remain separate from the direct installer checks. See [compatibility details](REBRANDING.md).
