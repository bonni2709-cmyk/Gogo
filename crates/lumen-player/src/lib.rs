//! Windows-only embedded HTML5 video window using the installed WebView2 runtime.
//! No CDN URL is handed to WebView2; its source is an authenticated Rust loopback endpoint.
use anyhow::Result;
use lumen_youtube::PlaybackSession;

pub fn supported() -> bool { cfg!(target_os = "windows") }

pub fn open(session: PlaybackSession) -> Result<()> {
    #[cfg(target_os = "windows")]
    {
        use std::{sync::mpsc, time::Duration};
        if windows::ACTIVE.swap(true, std::sync::atomic::Ordering::AcqRel) {
            anyhow::bail!("a player window is already open; close it before starting another video");
        }
        let active = windows::ActiveGuard;
        let (ready, receiver) = mpsc::sync_channel(1);
        std::thread::Builder::new().name("lumen-player".into()).spawn(move || { let _active = active; windows::run(session, ready); })?;
        receiver.recv_timeout(Duration::from_secs(15))
            .map_err(|_| anyhow::anyhow!("player initialization timed out or stopped"))?
            .map_err(anyhow::Error::msg)
    }
    #[cfg(not(target_os = "windows"))]
    {
        drop(session);
        anyhow::bail!("native playback is not implemented on this platform")
    }
}

#[cfg(target_os = "windows")]
mod windows {
    use super::*;
    pub static ACTIVE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    pub struct ActiveGuard;
    impl Drop for ActiveGuard {
        fn drop(&mut self) { ACTIVE.store(false, std::sync::atomic::Ordering::Release); }
    }
    use tao::{event::{Event, WindowEvent}, event_loop::{ControlFlow, EventLoopBuilder},
        platform::{run_return::EventLoopExtRunReturn, windows::EventLoopBuilderExtWindows}, window::WindowBuilder};
    use wry::WebViewBuilder;

    pub fn run(session: PlaybackSession, ready: std::sync::mpsc::SyncSender<std::result::Result<(), String>>) {
        let mut event_loop = EventLoopBuilder::new().with_any_thread(true).build();
        let window = match WindowBuilder::new().with_title("Lumen Tube · Video")
            .with_inner_size(tao::dpi::LogicalSize::new(960.0, 600.0)).build(&event_loop) {
            Ok(window) => window,
            Err(error) => { let _ = ready.send(Err(format!("player window unavailable: {error}"))); return; }
        };
        let local_url = session.local_url();
        // No JavaScript, remote documents, iframes, images or user-supplied HTML.
        let html = format!(r#"<!doctype html><html><head>
<meta http-equiv="Content-Security-Policy" content="default-src 'none'; media-src {local_url}; style-src 'unsafe-inline'; script-src 'none'; frame-src 'none'; connect-src 'none'; base-uri 'none'; form-action 'none'">
<meta name="referrer" content="no-referrer"><style>
html,body{{margin:0;height:100%;background:#101827;color:white;font-family:sans-serif}}
video{{width:100%;height:94%;object-fit:contain}}p{{margin:0 12px;font-size:12px}}
</style></head><body><video controls autoplay playsinline src="{local_url}"></video>
<p>Playback via local Rust gateway. Close this window to stop.</p></body></html>"#);
        let webview = match WebViewBuilder::new().with_html(html).with_devtools(false)
            .with_navigation_handler(|url| url == "about:blank")
            .with_new_window_req_handler(|_| false)
            .build(&window) {
            Ok(webview) => webview,
            Err(error) => { let _ = ready.send(Err(format!("WebView2 initialization failed (install Microsoft Edge WebView2 Runtime): {error}"))); return; }
        };
        if ready.send(Ok(())).is_err() { return; }
        event_loop.run_return(move |event, _, control_flow| {
            *control_flow = ControlFlow::Wait;
            if matches!(event, Event::WindowEvent { event: WindowEvent::CloseRequested, .. }) {
                *control_flow = ControlFlow::Exit;
            }
        });
        drop(webview);
        drop(window);
        drop(session);
    }
}
