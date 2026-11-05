slint::slint! {
    import { VerticalBox, Button } from "std-widgets.slint";
    export component AppWindow inherits Window {
        title: "Lumen Tube";
        width: 480px;
        height: 360px;
        background: #101827;
        in-out property <string> status: "Ready for automatic network self-test";
        in-out property <bool> busy: false;
        callback start();
        VerticalBox {
            padding: 24px;
            spacing: 16px;
            Text { text: "Lumen Tube"; color: #ffffff; font-size: 30px; }
            Text { text: "Network core preview · no manual transport settings"; color: #a7bfd8; wrap: word-wrap; }
            Text { text: root.status; color: #ffffff; wrap: word-wrap; }
            Button { text: "Check network and start proxy"; enabled: !root.busy; clicked => { root.start(); } }
            Text { text: "YouTube search and playback are not implemented in this preview."; color: #a7bfd8; wrap: word-wrap; }
        }
    }
}

pub fn run() -> Result<(), slint::PlatformError> {
    let app = AppWindow::new()?;
    let weak = app.as_weak();
    app.on_start(move || {
        let weak = weak.clone();
        if let Some(ui) = weak.upgrade() {
            ui.set_busy(true);
            ui.set_status("Testing TLS connectivity…".into());
        }
        std::thread::spawn(move || {
            let runtime = match tokio::runtime::Runtime::new() {
                Ok(runtime) => runtime,
                Err(error) => {
                    let message = error.to_string();
                    let _ = weak.upgrade_in_event_loop(move |ui| {
                        ui.set_status(message.into()); ui.set_busy(false);
                    });
                    return;
                }
            };
            runtime.block_on(async move {
                let result = async {
                    let selection = lumen_core::select().await?;
                    // Same port accepts CONNECT or SOCKS5. No external interfaces.
                    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
                    let address = listener.local_addr()?;
                    let message = format!("Verified: {:?}\nLocal proxy: {}\nPlayer integration pending", selection.strategy, address);
                    let _ = weak.upgrade_in_event_loop(move |ui| ui.set_status(message.into()));
                    lumen_proxy::serve(listener, selection.strategy).await
                }.await;
                if let Err(error) = result {
                    let message = format!("Network unavailable: {error:#}");
                    tracing::warn!(%message);
                    let _ = weak.upgrade_in_event_loop(move |ui| {
                        ui.set_status(message.into()); ui.set_busy(false);
                    });
                }
            });
        });
    });
    app.run()
}
