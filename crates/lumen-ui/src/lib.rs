use std::{rc::Rc, sync::Arc};

pub type PlayerLauncher = Arc<dyn Fn(lumen_youtube::PlaybackSession) -> anyhow::Result<()> + Send + Sync>;
use tokio::sync::mpsc;

slint::slint! {
    import { VerticalBox, Button, LineEdit, ListView } from "std-widgets.slint";
    export struct VideoRow { title: string, subtitle: string, id: string }
    export component AppWindow inherits Window {
        title: "Lumen Tube";
        width: 560px;
        height: 700px;
        background: #101827;
        in-out property <string> status: "Preparing automatic network self-test…";
        in-out property <bool> playback-supported: false;
        in-out property <bool> busy: false;
        in-out property <bool> connected: false;
        in-out property <bool> searching: false;
        in-out property <string> query;
        in-out property <string> search-status;
        in-out property <string> selected-video;
        in-out property <[VideoRow]> videos;
        callback start();
        callback search(string);
        callback choose(int);
        VerticalBox {
            padding: 20px;
            spacing: 12px;
            Text { text: "Lumen Tube"; color: #ffffff; font-size: 30px; }
            Text { text: root.status; color: #a7bfd8; wrap: word-wrap; }
            Button { text: "Retry network check"; enabled: !root.busy && !root.connected; clicked => { root.start(); } }
            LineEdit { text <=> root.query; placeholder-text: "Search YouTube"; enabled: root.connected && !root.searching; }
            Button { text: root.searching ? "Working…" : "Search"; enabled: root.connected && !root.searching; clicked => { root.search(root.query); } }
            Text { text: root.search-status; color: #ffffff; wrap: word-wrap; }
            ListView {
                for video[index] in root.videos : Rectangle {
                    height: 108px;
                    background: #1b293d;
                    VerticalBox {
                        padding: 8px;
                        spacing: 4px;
                        Text { text: video.title; color: #ffffff; overflow: elide; }
                        Text { text: video.subtitle; color: #a7bfd8; overflow: elide; }
                        Button { text: root.playback-supported ? "Play" : "Check stream"; enabled: root.connected && !root.searching; clicked => { root.choose(index); } }
                    }
                }
            }
            Text { text: root.selected-video; color: #a7bfd8; wrap: word-wrap; }
            Text { text: root.playback-supported ? "Windows WebView2 player · progressive MP4 preview" : "Stream diagnostics · Android native playback pending"; color: #a7bfd8; wrap: word-wrap; }
        }
    }
}

enum Command { Start, Search(String), Prepare(String) }

fn unavailable(weak: &slint::Weak<AppWindow>, message: String) {
    let _ = weak.upgrade_in_event_loop(move |ui| {
        ui.set_status(message.into());
        ui.set_busy(false);
        ui.set_connected(false);
        ui.set_searching(false);
    });
}

async fn worker(weak: slint::Weak<AppWindow>, mut commands: mpsc::Receiver<Command>, player: Option<PlayerLauncher>) {
    let mut proxy: Option<tokio::task::JoinHandle<anyhow::Result<()>>> = None;
    let mut search: Option<lumen_youtube::SearchClient> = None;
    loop {
        tokio::select! {
            command = commands.recv() => {
                let Some(command) = command else { break; };
                match command {
                    Command::Start => {
                        if search.is_some() { continue; }
                        let result = async {
                            let selection = lumen_core::select().await?;
                            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
                            let address = listener.local_addr()?;
                            let client = lumen_youtube::SearchClient::new(address)?;
                            Ok::<_, anyhow::Error>((selection, listener, address, client))
                        }.await;
                        match result {
                            Ok((selection, listener, address, client)) => {
                                proxy = Some(tokio::spawn(lumen_proxy::serve(listener, selection.strategy)));
                                search = Some(client);
                                let message = format!("Transport: {:?} · proxy {}", selection.strategy, address);
                                let _ = weak.upgrade_in_event_loop(move |ui| {
                                    ui.set_status(message.into());
                                    ui.set_busy(false);
                                    ui.set_connected(true);
                                });
                            }
                            Err(error) => unavailable(&weak, format!("Network unavailable: {error:#}")),
                        }
                    }
                    Command::Prepare(id) => {
                        let result = match search.as_ref() {
                            Some(client) => match tokio::time::timeout(std::time::Duration::from_secs(75), async {
                                if let Some(launch) = player.as_ref() {
                                    let stream = client.prepare_stream(&id).await?;
                                    let metadata = stream.metadata.clone();
                                    let session = lumen_youtube::PlaybackSession::start(client.clone(), stream).await?;
                                    // A dedicated Windows UI thread owns the session until its window closes.
                                    launch(session)?;
                                    Ok(metadata)
                                } else {
                                    client.prepare_video(&id).await
                                }
                            }).await {
                                Ok(result) => result,
                                Err(_) => Err(anyhow::anyhow!("media preparation timed out")),
                            },
                            None => Err(anyhow::anyhow!("network is not ready")),
                        };
                        let playing = player.is_some();
                        let _ = weak.upgrade_in_event_loop(move |ui| {
                            ui.set_searching(false);
                            match result {
                                Ok(video) => ui.set_selected_video(format!("{} · {}\n{}", video.title, video.quality, if playing { "Player window opened. Use its native controls to play/pause/seek." } else { "MP4 CDN byte verified. Android player integration pending." }).into()),
                                Err(error) => ui.set_selected_video(format!("Video is not prepared: {error:#}").into()),
                            }
                        });
                    }
                    Command::Search(query) => {
                        let result = match search.as_ref() {
                            Some(client) => client.search(&query).await,
                            None => Err(anyhow::anyhow!("network is not ready")),
                        };
                        let _ = weak.upgrade_in_event_loop(move |ui| {
                            ui.set_searching(false);
                            match result {
                                Ok(videos) => {
                                    ui.set_search_status(format!("{} video(s)", videos.len()).into());
                                    let rows: Vec<VideoRow> = videos.into_iter().map(|video| VideoRow {
                                        title: video.title.into(),
                                        subtitle: format!("{} · {}", video.channel, video.duration).into(),
                                        id: video.id.into(),
                                    }).collect();
                                    ui.set_videos(Rc::new(slint::VecModel::from(rows)).into());
                                }
                                Err(error) => ui.set_search_status(format!("Search failed: {error:#}").into()),
                            }
                        });
                    }
                }
            }
            result = async {
                match proxy.as_mut() {
                    Some(task) => task.await,
                    None => std::future::pending().await,
                }
            } => {
                proxy = None;
                search = None;
                unavailable(&weak, format!("Local proxy stopped: {result:?}"));
            }
        }
    }
    if let Some(proxy) = proxy { proxy.abort(); }
}

pub fn run() -> Result<(), slint::PlatformError> { run_with_player(None) }

pub fn run_with_player(player: Option<PlayerLauncher>) -> Result<(), slint::PlatformError> {
    use slint::Model;
    let app = AppWindow::new()?;
    app.set_playback_supported(player.is_some());
    let (sender, receiver) = mpsc::channel(4);
    let weak = app.as_weak();
    std::thread::spawn(move || {
        match tokio::runtime::Runtime::new() {
            Ok(runtime) => runtime.block_on(worker(weak, receiver, player)),
            Err(error) => unavailable(&weak, format!("Runtime unavailable: {error}")),
        }
    });
    let start_sender = sender.clone();
    let weak = app.as_weak();
    app.on_start(move || {
        if let Some(ui) = weak.upgrade() {
            if ui.get_busy() || ui.get_connected() { return; }
            if start_sender.try_send(Command::Start).is_ok() {
                ui.set_busy(true);
                ui.set_status("Testing TLS connectivity…".into());
            } else {
                ui.set_status("Network worker unavailable".into());
            }
        }
    });
    let weak = app.as_weak();
    let search_sender = sender.clone();
    app.on_search(move |query| {
        if let Some(ui) = weak.upgrade() {
            if !ui.get_connected() || ui.get_searching() { return; }
            if query.trim().is_empty() || query.chars().count() > 200 {
                ui.set_search_status("Enter 1–200 characters".into()); return;
            }
            if search_sender.try_send(Command::Search(query.to_string())).is_ok() {
                ui.set_searching(true);
                ui.set_search_status("Searching through the verified proxy…".into());
                ui.set_selected_video("".into());
                ui.set_videos(Rc::new(slint::VecModel::<VideoRow>::default()).into());
            } else {
                ui.set_search_status("Network worker unavailable".into());
            }
        }
    });
    let weak = app.as_weak();
    app.on_choose(move |index| {
        if let Some(ui) = weak.upgrade() {
            if !ui.get_connected() || ui.get_searching() { return; }
            if let Ok(index) = usize::try_from(index) {
                if let Some(video) = ui.get_videos().row_data(index) {
                    if sender.try_send(Command::Prepare(video.id.to_string())).is_ok() {
                        ui.set_searching(true);
                        ui.set_selected_video("Resolving progressive MP4 and checking CDN through proxy…".into());
                    } else {
                        ui.set_selected_video("Network worker unavailable".into());
                    }
                }
            }
        }
    });
    let startup = app.as_weak();
    slint::Timer::single_shot(std::time::Duration::ZERO, move || {
        if let Some(ui) = startup.upgrade() { ui.invoke_start(); }
    });
    app.run()
}
