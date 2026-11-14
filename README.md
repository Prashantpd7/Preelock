# Preelock — Local-First Focus, Productivity & Screen Time for macOS & Windows

Preelock is an open-source, local-first focus and productivity desktop application for macOS and Windows. Create focused work sessions, track app and website screen time, manage tasks, and enforce distraction blocking across applications and browsers.

There are no user accounts, no telemetry, and no cloud dependencies. Your data stays entirely on your computer.

---

## Key Features

- **Focus Sessions:** Standard and Strict focus modes, customizable timers, Pomodoro support, and emergency unlock challenges.
- **Screen Time Tracking:** Local-first, privacy-preserving application and website screen time metrics with daily aggregation.
- **Application Blocking:** Block distracting desktop apps with timed locks or daily time limits.
- **Website Blocking:** Block distracting websites across Safari, Google Chrome, Brave, Microsoft Edge, Comet, and Arc.
- **Adult Website Blocking:** Embedded system-wide filtering of ~50,000 adult domains across all major browsers with zero external network dependencies.
- **Strict Block Commitments:** Non-bypassable locks with durable 20-second activation countdowns that survive app closures and window navigation.
- **Background Engine:** Authoritative background daemon supervised by macOS LaunchAgent (`com.preelock.background`) and Windows Task Scheduler (`PreelockBackground`).
- **Safari Direct Integration:** Direct website blocking in Safari on macOS without requiring any browser extension.

---

## Architecture & Engineering Documentation

- [System Architecture & Component Map](ARCHITECTURE.md)
- [Security Policy & Threat Model](SECURITY.md)
- [Testing Guide & Quality Gates](TESTING.md)
- [Database Migrations & Storage Architecture](DATABASE_MIGRATIONS.md)
- [OS & Browser Compatibility Matrix](COMPATIBILITY.md)
- [Release Process & Artifact Checklist](RELEASE.md)
- [Developer & Coding Assistant Guidelines](AGENTS.md)
- [Changelog](CHANGELOG.md)

---

## Requirements & Supported Platforms

- **macOS:** macOS 12 (Monterey) through macOS 15+ (Sequoia) on Apple Silicon (`aarch64`) and Intel (`x86_64`).
- **Windows:** Windows 10 (21H2+) and Windows 11 (`x86_64`) with WebView2.

---

## Development & Building

### Prerequisites

- Node.js >= 24.13.0
- Rust >= 1.90.0

### Commands

```bash
# Install frontend dependencies
npm ci

# Run development desktop app
npm run desktop

# Run full test suite
npm run check
npm run rust:check
npm run rust:test

# Build production macOS application bundle
npm run desktop:build -- --bundles app
```

---

## License

MIT License. See [LICENSE](LICENSE) for details.
