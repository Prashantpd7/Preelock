"""Compare the installed NSIS payload with Tauri's restored compiler output.

Tauri patches the first __TAURI_BUNDLE_TYPE_VAR_UNK to ..._NSS when bundling,
then restores the unpatched build executable. Allow precisely that change.
See tauri-bundler/src/bundle.rs: patch_binary and bundle_project.
"""
from pathlib import Path
import sys


def verify(built: bytes, installed: bytes):
    marker = b"__TAURI_BUNDLE_TYPE_VAR_UNK"
    assert marker in built, "Tauri bundle-type marker missing from build output"
    expected = built.replace(marker, b"__TAURI_BUNDLE_TYPE_VAR_NSS", 1)
    assert installed == expected, "Installed executable differs beyond Tauri's NSIS marker"


if __name__ == "__main__":
    verify(Path(sys.argv[1]).read_bytes(), Path(sys.argv[2]).read_bytes())
    print("Installed executable matches the build with exactly Tauri's NSIS marker patch.")
