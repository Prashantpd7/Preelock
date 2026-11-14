# Preelock Operating System & Browser Compatibility Matrix

This document defines supported operating systems, hardware architectures, and browser integrations for Preelock.

---

## 1. Operating System Support Matrix

| Operating System | Architecture              | Supported Versions                              | Notes / Requirements                                                                     |
| :--------------- | :------------------------ | :---------------------------------------------- | :--------------------------------------------------------------------------------------- |
| **macOS**        | Apple Silicon (`aarch64`) | macOS 12 (Monterey) through macOS 15+ (Sequoia) | Native ARM64 binary; LaunchAgent background daemon; AppKit / Accessibility integration.  |
| **macOS**        | Intel (`x86_64`)          | macOS 12 (Monterey) through macOS 15+           | Native x86_64 binary; LaunchAgent background daemon; AppKit / Accessibility integration. |
| **Windows**      | x64 (`x86_64`)            | Windows 10 (21H2+) and Windows 11               | Requires WebView2 runtime (pre-installed on Win 10/11); Task Scheduler background task.  |
| **Windows**      | ARM64                     | Windows 11 ARM                                  | Runs via x64 emulation on ARM64 Windows.                                                 |
| **Linux**        | x86_64                    | Not Supported                                   | Preelock is macOS and Windows native. CI uses Ubuntu for headless web checks only.       |

---

## 2. Browser Support & Blocking Matrix

| Browser            | Platform        | Blocking Technology                          | Needs Extension?                    | Safari Normal Support | Adult Website Blocking |
| :----------------- | :-------------- | :------------------------------------------- | :---------------------------------- | :-------------------- | :--------------------- |
| **Safari**         | macOS           | AppleScript & macOS Accessibility API        | **No** (Direct native)              | Yes                   | Yes (System-wide)      |
| **Google Chrome**  | macOS & Windows | declarativeNetRequest & Native UI Automation | Yes (Recommended) / Native fallback | N/A                   | Yes (System-wide)      |
| **Brave Browser**  | macOS & Windows | declarativeNetRequest & Native UI Automation | Yes (Recommended) / Native fallback | N/A                   | Yes (System-wide)      |
| **Microsoft Edge** | macOS & Windows | declarativeNetRequest & Native UI Automation | Yes (Recommended) / Native fallback | N/A                   | Yes (System-wide)      |
| **Comet / Arc**    | macOS & Windows | declarativeNetRequest & Native UI Automation | Yes (Recommended) / Native fallback | N/A                   | Yes (System-wide)      |
| **Firefox**        | macOS & Windows | App tracking (Window minimization)           | No (App lock only)                  | N/A                   | Under investigation    |

---

## 3. Platform Differences & Parity

1. **Background Supervision:**
   - **macOS:** Supervised via user LaunchAgent (`~/Library/LaunchAgents/com.preelock.background.plist`). Automatically launches on login and restarts if killed.
   - **Windows:** Supervised via Task Scheduler interactive user task (`PreelockBackground`) or MSIX startup task.
2. **Native Website Redirects:**
   - **macOS:** Dispatches AppleScript to active tab of front window (`about:blank`).
   - **Windows:** Uses Windows UI Automation (`IUIAutomationValuePattern`) to set address bar and navigate to `about:blank`.
3. **Camera Presence (Optional):**
   - **macOS:** Uses `AVFoundation` and Apple `Vision` framework for on-device face/person detection.
   - **Windows:** Uses Windows Media Capture face analysis.
