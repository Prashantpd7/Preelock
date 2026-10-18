# Contributing

Keep changes focused and local-first. Validate user input in Rust, preserve database compatibility, and keep timers and policy enforcement out of React. Add meaningful tests for domain changes and persistence failures. Never silently replace a damaged database.

Before submitting:

```sh
npm run check
npm run format:check
npm run rust:check
npm run rust:test
```

Include the platform and toolchain used, test evidence, and known limitations. Test native behavior on both Windows and macOS before claiming cross-platform support. Use versioned migrations, never rewrite a shipped migration. Do not commit keys, local data, build products or generated schema output. Commit package-lock.json and src-tauri/Cargo.lock.

New monitoring or enforcement features require visible consent, capability checks, accessible controls, retention policy and tested emergency recovery. Do not add simulated statistics or controls that suggest unimplemented capabilities are active. Follow system reduced-motion preferences and test both themes and keyboard navigation.
