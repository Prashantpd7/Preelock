# Developer & AI Assistant Instructions (`AGENTS.md`)

This file contains mandatory instructions for Gemini, coding assistants, and human engineers working on the Preelock codebase.

---

## 1. Core Directives

1. **Maintain Working Features:** Never delete or degrade existing working features (Screen Time, App Locks, Website Blocking, Safari integration, Adult Website Blocking, Strict Mode, Focus Sessions, Tasks, Settings).
2. **Never Invent Tests:** Do not report that tests passed without executing them. Run tests through actual terminal commands.
3. **No Unrelated Redesigns:** Do not change UI styling, layouts, or component structures unless directly requested by the user.
4. **Preserve User Data:** Every database schema change must be additive, atomic (`BEGIN IMMEDIATE; ... COMMIT;`), and backward-compatible. Never drop user tables or delete user sessions.
5. **Cross-Platform Awareness:** macOS and Windows are both tier-1 platforms. Always respect conditional compilation (`#[cfg(target_os = "macos")]` / `#[cfg(target_os = "windows")]`).

---

## 2. Standard Workflow for Any Code Modification

Follow this exact 10-step sequence for every future modification:

1. **Check Git Status:**
   Run `git status` to inspect current branch, uncommitted files, and working tree before making any change.
2. **Understand the Architecture:**
   Read `ARCHITECTURE.md`, `SECURITY.md`, and relevant source files before touching code.
3. **Identify Side Effects:**
   Determine whether the change impacts `--background`, the GUI, IPC, SQLite, or browser extensions.
4. **Make Minimal, Scoped Edits:**
   Only modify the files strictly required to implement the fix or feature.
5. **Frontend Quality Check:**
   Run:
   ```bash
   npm run check
   npm run format:check
   ```
   (Must pass typecheck, linting with 0 errors/warnings, all Vitest tests, and formatting).
6. **Rust Quality Check:**
   Run:
   ```bash
   npm run rust:check
   npm run rust:test
   ```
   (Must pass rustfmt, clippy with `-D warnings`, and all 395+ unit tests).
7. **Native macOS Probe (on macOS):**
   Run:
   ```bash
   npm run native:probe
   ```
8. **Build Desktop Bundle:**
   Run:
   ```bash
   npm run desktop:build -- --bundles app
   ```
9. **Inspect Git Diff:**
   Run `git diff` to verify that no unrelated code or tests were removed or broken.
10. **Install & Verify Locally:**
    Verify the installed binary in `/Applications/Preelock.app` or Windows executable before declaring completion.
