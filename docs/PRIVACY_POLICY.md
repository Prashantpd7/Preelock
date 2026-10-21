# Preelock privacy policy (draft)

> **Draft for the Microsoft Store listing.** Publish this text at a public URL
> you control and replace the bracketed placeholders before submitting. Do not
> invent a URL, support address, or company details.

Effective date: [EFFECTIVE DATE]
Applies to: the Preelock desktop application for Windows and macOS, including the
bundled browser extension.
Contact: [SUPPORT EMAIL OR SUPPORT URL]

## Summary

Preelock is local-first. It has no account system, no cloud backend, no analytics
or telemetry service, and no advertising. The developer does not receive your
sessions, tasks, settings, activity data, camera data, or browser activity. Data
is stored on your own computer and is only shared if you export it or send it
yourself.

## What Preelock stores on your computer

- Focus sessions: title, duration, status, remaining and focused time, reports
  and event history.
- Tasks you create, and preferences such as theme, reminders and daily goal.
- Attention records created during sessions you start with activity tracking
  enabled: an anonymous interval record containing the foreground application
  identifier (for example an executable path), whether you were idle, and the
  classification and duration. Window titles, keystrokes and screen contents are
  not captured.
- A random local pairing token used to authenticate the browser extension.
- Files you choose to export (JSON backup, CSV history) at the location you pick.

Location: the per-user application-data folder for `com.vystriq.desktop`
(for example `%APPDATA%\com.vystriq.desktop` on Windows and
`~/Library/Application Support/com.vystriq.desktop` on macOS). In a Microsoft
Store install, Windows may keep newly created files in the package's private
application-data store. The bundled browser extension is copied to a folder the
browser can read: `%USERPROFILE%\Preelock\browser-extension` (existing installations retain
`%USERPROFILE%\VYSTRIQ\browser-extension` to preserve browser registration) for a packaged
Windows build, otherwise the application-data folder.

The database is not included in the installer or Store package, and a new
computer or new OS user starts with empty data.

## Camera

Camera presence checking is available on macOS only. It is off by default, is
enabled per session or for an explicit camera test, and requires the operating
system camera permission. Frames are analysed in memory on your device to decide
whether a person is present. Preelock does not record, save, or upload images or
video. Windows builds report camera presence as unavailable and do not open the
camera.

## Browser extension

The bundled browser extension communicates only with the Preelock app on your
computer over an authenticated loopback connection (`127.0.0.1`). It sends
heartbeats containing a local client identifier, the browser name, the applied
policy identifier, and block/error status. Website allow/block lists and pairing
state stay on your computer. The extension does not send browsing history to the
developer and does not use a remote filtering service. Filtering is limited to
the browser profile where you installed and paired it.

## Notifications, sound, and startup

Notifications and sounds are produced locally by the operating system.
"Open at login" / "Open at sign-in" is controlled by you and by operating-system
settings; Preelock never enables it on its own.

## What Preelock does not do

- No account, login, cloud database, or data sync.
- No telemetry, crash reporting, usage analytics, or advertising identifiers.
- No sale, rent, or sharing of personal data with third parties.
- No reading of window titles, keystrokes, clipboard, or screen contents.
- No administrator service, driver, firewall, DNS, proxy, or VPN changes.

## Retention and deletion

Records stay until you delete them in the app, delete the application-data
folder, or remove the app. On macOS and on the direct (NSIS) Windows download,
uninstalling normally leaves local data for a later reinstall; delete the
application-data folder yourself for a complete removal. For a Microsoft Store
install, Windows may remove the package's private data when the app is
uninstalled, while data inherited from an earlier direct installation is kept.
Use the in-app backup before uninstalling if you want a copy.

Deleting a session in the app removes it from the database. Uninstalling the
browser extension removes its stored pairing and policy state.

## Children

Preelock is a general productivity tool and is not directed at children. It does
not knowingly collect personal information from anyone, because it does not
collect data centrally at all.

## Security

Local files are protected by your operating-system user account permissions.
The browser bridge accepts only authenticated local requests. No method of
storage is completely secure; keep your OS account and disk protections in place.

## Changes

If this policy changes, the updated text will be published at the same URL with a
new effective date, and material changes will be noted in the app's release
notes.

## Contact

Questions about this policy: [SUPPORT EMAIL OR SUPPORT URL]
