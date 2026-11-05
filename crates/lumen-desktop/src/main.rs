fn main() -> Result<(), slint::PlatformError> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();
    if lumen_player::supported() {
        lumen_ui::run_with_player(Some(std::sync::Arc::new(lumen_player::open)))
    } else {
        lumen_ui::run()
    }
}
