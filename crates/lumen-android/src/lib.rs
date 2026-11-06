//! Slint NativeActivity plus a Gradle Java host for Media3 playback. No VpnService yet.
#[cfg(target_os = "android")]
mod player;

#[cfg(target_os = "android")]
#[no_mangle]
fn android_main(app: android_activity::AndroidApp) {
    slint::android::init(app).expect("initialize Android UI backend");
    let result = if player::available() {
        lumen_ui::run_with_player(Some(std::sync::Arc::new(player::open)))
    } else {
        // Legacy cargo-apk NativeActivity package has no Java host: diagnostics only.
        lumen_ui::run()
    };
    if let Err(error) = result { eprintln!("Lumen Tube UI error: {error}"); }
}
