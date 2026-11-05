//! NativeActivity entry point. VpnService/TUN and Media3 are not implemented yet.
#[cfg(target_os = "android")]
#[no_mangle]
fn android_main(app: android_activity::AndroidApp) {
    slint::android::init(app).expect("initialize Android UI backend");
    if let Err(error) = lumen_ui::run() {
        eprintln!("Lumen Tube UI error: {error}");
    }
}
