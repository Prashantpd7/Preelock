# Architecture

React renders the Rust-owned session snapshot through serialized IPC calls and Zustand. Native commands run off the event loop; AppKit calls dispatch to the main thread. The Rust worker runs independently of React once per second. No scripts, shell commands or arbitrary native paths can be executed through IPC.

## Session and enforcement lifecycle

A configuration contains mode, selected app identities, domain rules, browser identity, internet policy, activity consent and camera consent. Inputs are validated in Rust. Legacy records deserialize to Standard mode with monitoring off. Mode policy is enforced in Rust, including rejection of ordinary pause/end in committed sessions. Deep Focus starts are rejected until a managed implementation exists.

Browser-dependent sessions follow `ready → preparing → running → completed/ended/interrupted`; Standard sessions may pause. Preparation persists intent before publishing rules. The extension installs session rules and acknowledges the session ID before the timer runs. Preparation times out after 45 seconds. Strict sessions only exit early through a typed emergency confirmation or a detected enforcement failure. Restart preserves productive totals without adding offline credit, then revalidates the browser; no active restriction is assumed from an old status. Preparation time itself does not count as focus time.

The engine uses cloned state plus transactional SQLite writes before publishing a new network policy. Schema v2 adds a separate audit table while retaining the v1 JSON model tables. Activity aggregates checkpoint every five seconds and on user actions/transitions; a crash may lose the last few seconds of samples. The UI retains the most recent 200 events per session; the audit table retains all committed events. No raw video, page URLs, titles, messages or keystrokes are stored. The database is local and unencrypted.

A persistence failure stops native monitoring/capture and stops publishing restriction leases. The failed process refuses further session mutations until restarted. The database might still contain the last successful active checkpoint: recovery reconciles its timer and requires new browser acknowledgement. There are no persistent OS locks to undo. Logging avoids activity content and pairing secrets.

## Timing and activity

macOS `mach_continuous_time` and Windows `GetTickCount64` measure sample deltas including sleep. Only the Rust worker can credit productive time; UI polls cannot. Gaps over 3 seconds become RECOVERING with zero credit. Restart records a nonnegative wall-clock gap as unknown elapsed, never productive; clock changes cannot advance the target. Historical elapsed timers remain labeled legacy and active legacy timers stop on upgrade.

`attention.rs` combines approved foreground apps, fresh paired-browser hostname metadata, idle context, optional Vision presence and configured fallback. Two stable seconds precede accepted focus. Grace delays confident distraction/absence labels, but never awards uncertain time. Reading/lecture permit 20 minutes idle; other tasks permit five, unless camera presence supports continued work. This does not establish comprehension or defeat input simulation. Manual policy accepts at most 60 seconds after explicit self-report. Corrections require a note, cannot credit gaps or double-credit intervals, and stay auditable. Migration 3 stores indexed intervals; snapshots keep the latest 300 for review. Writes occur on transitions and five-second checkpoints.

Window close hides the window and preserves the worker. A visible tray/menu provides reopen and quit/release. Normal quit interrupts and saves active sessions; crashes preserve the last checkpoint for conservative recovery. There is no startup agent or hidden persistence.

Activity totals count observed one-second samples only; sleep, suspension and process downtime do not invent app activity. Idle means at least 60 seconds without OS input. Timer duration is not proof of attention. Analytics continue to assign focus to the local start date. All app sampling requires per-session consent. The app list is read only during setup/refresh; app contents are never inspected.

## Native adapters

macOS uses AppKit NSWorkspace/NSRunningApplication to enumerate regular running applications, observe the foreground bundle ID and hide a disallowed app. Windows uses EnumWindows, GetForegroundWindow, QueryFullProcessImageNameW, GetLastInputInfo and ShowWindowAsync to minimize a disallowed foreground window. These are cooperative UI controls, not process termination or launch prevention. Recovery applications are exempt. Errors interrupt enforcement visibly. Hidden/minimized windows can be reopened normally; cleanup stops further redirection rather than opening every window automatically.

Optional macOS presence capture uses AVFoundation and a local Vision upper-body detection request at approximately one frame per second, confidence threshold 0.6. Thirty seconds without detection reports extended absence. No frame leaves process memory. Capture starts only with OS authorization plus session consent and stops on consent withdrawal, pause, completion, interruption or process exit. A camera error disables capture visibly without pretending attention is verified. Windows presence detection and phone-use detection are unavailable.

## Browser isolation and recovery

The MV3 extension uses Chromium declarativeNetRequest session rules. It never reads page content. Blocklists use request domains; Strict allows selected domains and their subdomains; browser-offline blocks HTTP(S)/WS(S) except loopback. It cannot revoke cached/offline content or already established streams, filter other profiles/clients, prevent extension removal, or enforce a system-wide network policy.

An authenticated loopback bridge accepts bounded POSTs from extension origins with an exact Host header and random 64-character pairing token. No arbitrary OS actions are exposed. One extension installation/client pairs per process; pairing identity is not cryptographic proof that it is the same browser selected in the wizard. The user must select the paired browser/profile. A malicious same-user process with access to app data can spoof the bridge; this is not an adversarial security boundary.

Rules receive leases no longer than 65 seconds, refreshed by Desktop. A browser heartbeat older than 45 seconds interrupts the session. The extension polls while awake and uses 30-second browser alarms as a fallback. Failed bridge fetches clear its rules. Browser sleep/worker scheduling may delay cleanup beyond lease expiry; independent popup recovery, extension disable/uninstall and browser restart remain available. Session rules are cleared by a browser restart. The bridge never changes firewall, DNS, proxy or VPN configuration. See docs/FOCUS_MODE.md for exact bypass limits and setup.
