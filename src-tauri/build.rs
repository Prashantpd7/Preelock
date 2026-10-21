fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        cc::Build::new()
            .file("native/macos.m")
            .flag("-fobjc-arc")
            .flag("-fblocks")
            .compile("preelock_native");
        for framework in [
            "AppKit",
            "AVFoundation",
            "Vision",
            "UserNotifications",
            "ServiceManagement",
            "CoreGraphics",
            "CoreMedia",
            "CoreVideo",
        ] {
            println!("cargo:rustc-link-lib=framework={framework}");
        }
        println!("cargo:rerun-if-changed=native/macos.m");
    }
    tauri_build::try_build(tauri_build::Attributes::new().app_manifest(
        tauri_build::AppManifest::new().commands(&[
            "snapshot",
            "settings_action",
            "execute",
            "capabilities",
            "focus_setup",
            "open_setup",
            "request_camera",
            "reset_browser_pairing",
        ]),
    ))
    .expect("Tauri build configuration failed");
}
