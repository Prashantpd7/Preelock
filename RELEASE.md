# Preelock Release Process & Quality Gates

This document defines the strict, reproducible procedure for cutting and publishing releases of Preelock.

---

## 1. Release Quality Gates (Mandatory)

A release must **never** be cut or published unless the following gates pass 100%:

- [ ] `npm run check`: Typecheck, ESLint (0 errors, 0 warnings), Vitest (all 54 test files pass), and production frontend build pass.
- [ ] `npm run format:check`: Prettier check reports 0 unformatted files.
- [ ] `npm run rust:check`: `cargo fmt --check` and `cargo clippy --all-targets -- -D warnings` pass with 0 warnings.
- [ ] `npm run rust:test`: All 395+ Rust tests pass cleanly.
- [ ] `npm run native:probe`: Native macOS probe binary compiles and executes cleanly.
- [ ] Database migrations: Version check passes, transactional integrity verified, upgrade test from v1 succeeds.
- [ ] No hardcoded secrets, test credentials, or development URLs in source tree.

---

## 2. Release Artifacts Checklist

Every official release includes:

1. `Preelock-macOS-Apple-Silicon.dmg` (macOS arm64 bundle).
2. `Preelock-macOS-Intel.dmg` (macOS x86_64 bundle).
3. `Preelock-Windows-Setup.exe` (Windows NSIS installer).
4. `Preelock-Windows-MSIX.msix` (Windows Store package).
5. `preelock-browser-extension-v{version}.zip` (Chromium MV3 extension bundle).
6. `SHA256SUMS.txt` (Cryptographic verification checksums for all release binaries).

---

## 3. Step-by-Step Release Procedure

1. **Verify Git State:**
   Ensure `git status` is clean on branch `main` and all commits are pushed:

   ```bash
   git status
   git diff --stat
   ```

2. **Update Version:**
   Bump the version in `package.json`, `src-tauri/tauri.conf.json`, and `src-tauri/Cargo.toml`:

   ```bash
   node scripts/release-version.mjs <new-version>
   ```

3. **Update Documentation & Changelog:**
   Add entry to `CHANGELOG.md` with features, bug fixes, and security improvements.

4. **Run Full Local Test Suite:**

   ```bash
   npm run check
   npm run rust:check
   npm run rust:test
   npm run native:probe
   ```

5. **Build Release Bundles:**

   ```bash
   npm run desktop:build -- --bundles app
   npm run extension:zip
   ```

6. **Create Git Tag & Trigger Release Workflow:**

   ```bash
   git add .
   git commit -m "release: v<new-version>"
   git tag "v<new-version>"
   git push origin main --tags
   ```

7. **Verify Release Workflow in GitHub Actions:**
   - Ensure `.github/workflows/release.yml` completes successfully on all runners.
   - Verify generated checksums in `SHA256SUMS.txt`.
   - Download and smoke-test the production artifacts locally before making the release public.
