# Focus Mode: setup, limits and recovery

## What is implemented

| Capability                                 | macOS                                         | Windows                               |
| ------------------------------------------ | --------------------------------------------- | ------------------------------------- |
| Standard selected-app redirection          | Hide foreground user application              | Minimize foreground user window       |
| Strict approved-app redirection            | Hide unapproved user applications             | Minimize unapproved user windows      |
| Browser blocklist / allowlist / offline    | Paired Chromium extension                     | Same extension                        |
| Foreground app and idle summaries          | Native, consented                             | Native, consented; runtime unverified |
| Camera presence estimate                   | Local AVFoundation + Vision, consent required | Unavailable                           |
| Managed Deep Focus / restricted account    | Not provisioned; unavailable                  | Not provisioned; unavailable          |
| System-wide internet / VPN / proxy control | Unavailable                                   | Unavailable                           |

Hiding/minimizing does not terminate an app, stop its background network activity, or prevent it from being reopened. A running session redirects it again on a subsequent sample. Finder/Explorer, Settings and recovery tools remain accessible. Application selection lists currently running apps; open a desired app and refresh. Apps without an identifiable regular foreground window may be unobservable or uncontrollable.

## Browser setup

1. Run the desktop app. Open Settings → Permissions & focus setup.
2. In Chrome or Edge, open the Extensions manager, enable Developer mode and choose **Load unpacked**. Select this repository's `browser-extension` directory. Chromium 120 or newer is required. Brave/Vivaldi are accepted by identity but have not been runtime-tested; Firefox and Safari integrations are unavailable.
3. Use **Show pairing token** in Desktop. Paste it into the extension popup and select **Pair / reconnect**. The token is local authorization, not a cloud API key. Do not share it.
4. With no active session, select **Test installed browser matcher**. It runs six real Chromium `testMatchOutcome` checks against rules scoped to a synthetic tab ID and removes its test rules. It does not navigate or contact test domains. The check validates the browser matcher, not all possible bypasses.
5. Use **Retry checks** in Desktop. Ensure the extension is connected. In the session wizard choose that same browser; pair only one profile. If a second profile is rejected, reset pairing in Desktop after ending the session and reconnect the intended profile.
6. Enter bare domains, one per line or comma-separated. Subdomains are included. URLs, paths, ports, IPs and wildcard input are rejected. Localhost stays exempt for development and recovery.
7. Create and start the session. Browser-dependent sessions remain **preparing** until rule installation is acknowledged. **Sync now** in the popup accelerates activation. A timeout records interruption instead of starting an unenforced session.

Standard blocks selected domains. Strict defaults to a domain allowlist. Unchecking browser internet selects browser-offline mode. Coding/lecture presets are editable starting points; real platforms may require extra authentication, CDN and media domains. No preset guarantees compatibility.

The extension's block counter uses unpacked-extension debug feedback. Zero means no observed matches; it does not prove no access attempts occurred. Only counts are reported, never requested URLs.

## Consent and permissions

Activity sampling requires explicit per-session consent, also required for application redirection. The sidebar and active session disclose it. No Accessibility, Screen Recording or Input Monitoring permission is requested by the current adapters; unavailable native API access is reported rather than bypassed.

On macOS, use **Request camera permission**, respond to the OS prompt, then retry checks. Granting permission does not start capture. Enable camera presence in a session's review step. The packaged app includes NSCameraUsageDescription and the camera entitlement. If denied, use System Settings → Privacy & Security → Camera. **Stop camera** revokes that session's camera consent immediately. Native camera/analysis errors disable capture and appear in the event history. Detection may be wrong because of lighting, framing, occlusion, hardware or model behavior. Presence is not evidence of attention, comprehension or learning. No phone-use detection is claimed.

## Enforcement boundaries and bypasses

- Only the paired browser profile's new matching network requests are filtered. Other profiles, incognito, browsers and network clients are not filtered. Strict redirection excludes known alternate browser apps from approval, but cannot identify every renamed/custom browser.
- DNS changes, encrypted DNS, VPNs and OS proxies do not change the URL that browser rules match. However, an allowed remote proxy site, remote desktop, browser extension, embedded browser, local proxy, cached/offline page or existing connection can bypass the intended experience. This is not VPN-resistant or system-wide blocking.
- Disconnecting or changing networks does not alter local browser rules. If the extension/bridge cannot communicate, rules are released on the next extension check and the desktop records lost enforcement. Restriction loss is not detected instantaneously.
- The same-user administrator can quit VYSTRIQ, remove the extension, change files/clock, or use recovery tools. No anti-uninstall mechanism, service, kernel hook, security-tool termination or hidden persistence is installed.
- Browser alarms can be delayed. Lease expiry is checked when the extension wakes; it is not an OS-enforced exact cleanup deadline.
- System clock changes during a running process do not move the native timer. Productive targets never receive offline time. Restart and monitoring gaps are recorded as unknown/recovering; changing the clock cannot earn productive credit.

## Recovery and uninstall

Use **Emergency exit**, type `END FOCUS`, and confirm. The session is recorded as interrupted; app redirection and camera capture stop. Browser rules clear when the extension syncs. Settings shows pending cleanup until it receives acknowledgement.

If Desktop is unavailable: open the extension popup's emergency recovery, type `END FOCUS`, and release. Alternatively disable/remove the extension, or restart the browser (rules are session scoped). Quit/force-quit VYSTRIQ to stop native app redirection and camera capture. Reopen hidden/minimized apps from the Dock/taskbar. No DNS, firewall, VPN, proxy or system account changes need undoing.

For uninstall, first recover/end the session, remove the extension and quit VYSTRIQ. Remove the application bundle or use the Windows uninstaller. Keeping the application data directory preserves your history; back it up before deleting it. The local SQLite database and browser pairing token are in the platform app-data directory documented in DEVELOPMENT.md. No scheduled task, launch agent, Windows service or background helper installation is created.

## Managed policy prerequisites, intentionally not applied

A future Deep Focus deployment needs a restricted user account, managed policy with explicit scope, a rollback journal, and an independent administrator recovery account. Normal app privileges cannot offer universal administrator-proof restriction.

Windows AppLocker/App Control policies are supported administrative mechanisms, but must be designed and tested against the installed OS, policy deployment and recovery configuration. Current Windows 10/11 AppLocker applicability should be checked against Microsoft's [requirements](https://learn.microsoft.com/en-us/windows/security/application-security/application-control/app-control-for-business/applocker/requirements-to-use-applocker). This build does not write those policies or treat a detected Windows edition as proof of enforcement.

Apple documents managed [Parental Controls payloads](https://support.apple.com/guide/deployment/parental-controls-payload-settings-dep1c778f77/web) and controlled [Family Controls entitlement requests](https://developer.apple.com/documentation/familycontrols/requesting-the-family-controls-entitlement). Availability differs across platforms/OS versions and deployments; an ordinary unsigned macOS app must not assume access. This build uses documented [NSRunningApplication hiding](<https://developer.apple.com/documentation/appkit/nsrunningapplication/hide()>) only. It does not install MDM profiles, restricted accounts or privileged Network Extensions.

The extension uses [Chromium declarativeNetRequest](https://developer.chrome.com/docs/extensions/reference/api/declarativeNetRequest). Presence estimates use Apple's [VNDetectHumanRectanglesRequest](https://developer.apple.com/documentation/vision/vndetecthumanrectanglesrequest). These implementations provide the disclosed, limited controls; they do not substitute for managed OS application/network enforcement.

## Productive-time setup (v0.3)

Choose task context and an acceptance policy. Activity-only requires explicit sampling consent plus productive apps (Standard) or approved apps (Strict). Select the paired browser and productive domains for browser work; unidentified/unpaired browser activity earns no time. Reload the unpacked extension after upgrading and accept its tabs permission. Only a hostname is sent transiently to the authenticated local bridge; it is not persisted in session history. Camera-plus-activity requires authorized camera consent or an explicitly selected activity-only fallback. Missing evidence otherwise produces UNKNOWN.

The productive countdown pauses without releasing restrictions during DISTRACTED, ABSENT, UNKNOWN and RECOVERING. Manual checkpoints and interval corrections are labeled self-report. Original state totals retain corrections, so categories overlap corrected accepted time. Elapsed study time excludes explicit Standard pauses/preparation, includes monitoring gaps, and is attributed to session start day in daily reports. Normal window closure keeps monitoring running; use the VY menu bar/system tray to reopen or quit and release.

Presence uses a real Apple Vision upper-body model (confidence threshold 0.6, about 1 Hz) with 30-second absence detection plus configured grace and idle evidence. Face/head direction, phone detection, playback, file-change integrations and comprehension questions are not implemented. No typing-rate score is used. Input simulation, cached pages, other profiles and malicious extension reports are not prevented.

## v0.3.1 quick connection

Open Settings → Browser extension → Set up. Comet is now detected alongside supported Chromium browsers. Open Extensions, enable Developer mode, then Load unpacked from the folder shown by VYSTRIQ. Use Copy folder path and ⌘⇧G in the Mac folder picker. Open the extension popup, paste Copy connection code, click Connect, then Test blocking. Keep the desktop app open. The extension is exported outside the app bundle so its folder stays stable across rebuilds. Reload it after updates.

Strict sessions using only desktop apps can run without a browser connection. Website sessions must connect before continuing. Camera is optional: Allow camera → approve macOS prompt → Check again. If no prompt appears, use Open camera settings and enable VYSTRIQ.
