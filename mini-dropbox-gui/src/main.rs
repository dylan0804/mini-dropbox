use std::{ops::ControlFlow, path::PathBuf, sync::Arc};

use anyhow::anyhow;
use eframe::CreationContext;
use egui::{vec2, Align2, Vec2};
use egui_toast::{Toast, ToastOptions, Toasts};
use futures_util::{SinkExt, StreamExt};
use names::{Generator, Name};
use reqwest::Client;
use rfd::FileDialog;
use tokio::sync::mpsc::{self, Receiver, Sender};
use tokio_tungstenite::{
    connect_async,
    tungstenite::{Bytes, Message, Utf8Bytes},
};

use crate::{events::AppEvent, state::AppState};

mod events;
mod state;
mod toast;

#[tokio::main]
async fn main() -> eframe::Result {
    // env_logger::init(); // Log to stderr (if you run with `RUST_LOG=debug`).

    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_resizable(true),
        ..Default::default()
    };

    eframe::run_native(
        "Mini Dropbox",
        native_options,
        Box::new(|cc| Ok(Box::new(MyApp::new(cc)))),
    )
}

pub struct MyApp {
    app_state: AppState,
    nickname: String,
    reqwest_client: Arc<Client>,
    toasts: Toasts,
    files: Vec<PathBuf>,
    tx: Sender<AppEvent>,
    rx: Receiver<AppEvent>,
}

impl MyApp {
    fn new(_cc: &CreationContext) -> Self {
        let (tx, rx) = mpsc::channel::<AppEvent>(100);

        let toasts = Toasts::new()
            .anchor(Align2::RIGHT_TOP, (-10., 10.))
            .order(egui::Order::Tooltip);

        Self {
            reqwest_client: Arc::new(reqwest::Client::new()),
            app_state: AppState::OnStartup,
            files: vec![],
            nickname: "".into(),
            toasts,
            rx,
            tx,
        }
    }
}

impl eframe::App for MyApp {
    fn update(&mut self, ctx: &eframe::egui::Context, frame: &mut eframe::Frame) {
        egui::TopBottomPanel::bottom("footer").show(ctx, |ui| {
            ui.add_space(5.0);

            ui.allocate_ui_with_layout(
                ui.available_size(),
                egui::Layout::right_to_left(egui::Align::LEFT),
                |ui| {
                    ui.label(&self.nickname);
                },
            );

            ui.add_space(5.0);
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            // error handler
            while let Ok(app_event) = self.rx.try_recv() {
                match app_event {
                    AppEvent::WebSocketFailed(e) => {
                        self.show_error_toast(e.to_string());
                    }
                    AppEvent::WebSocketSuccess => {
                        self.app_state = AppState::Ready;
                    }
                }
            }

            match &self.app_state {
                AppState::OnStartup => {
                    let mut generator = Generator::with_naming(Name::Numbered);
                    self.nickname = generator.next().unwrap_or("Guest".into());

                    let tx = self.tx.clone();
                    tokio::spawn(async move {
                        let ws_stream = match connect_async("ws://127.0.0.1:3000/ws").await {
                            Ok((stream, _)) => stream,
                            Err(e) => {
                                tx.send(AppEvent::WebSocketFailed(anyhow!(
                                    "Websocket connection failed: {e}"
                                )))
                                .await
                                .ok();
                                return;
                            }
                        };

                        let (mut sender, mut receiver) = ws_stream.split();

                        tokio::spawn(async move {
                            if let Some(Ok(msg)) = receiver.next().await {
                                if process_message(msg).is_break() {
                                    tx.send(AppEvent::WebSocketSuccess).await.ok();
                                }
                            }
                        });
                    });

                    self.app_state = AppState::Connecting;
                }
                AppState::Connecting => {
                    ui.centered_and_justified(|ui| {
                        ui.add_space(ui.available_height() / 2.);
                        ui.vertical_centered_justified(|ui| {
                            ui.add(egui::Spinner::new().size(32.));
                            ui.label("Connecting...");
                        })
                    });
                }
                AppState::Ready => {
                    if ui.button("Select file").clicked() {
                        let files = FileDialog::new().set_directory("/").pick_file().unwrap();
                        self.files.push(files);
                    }

                    if !self.files.is_empty() {
                        self.files.iter().for_each(|p| {
                            ui.horizontal(|ui| {
                                ui.label(p.file_name().unwrap().to_string_lossy());
                                ui.button("Send to");
                            });
                        })
                    }
                }
            }

            self.toasts.show(ctx);
        });
    }
}

fn process_message(msg: Message) -> ControlFlow<(), ()> {
    match msg {
        Message::Ping(_) => ControlFlow::Break(()),
        Message::Text(b) => {
            println!("{}", b.as_str());
            ControlFlow::Continue(())
        }
        _ => ControlFlow::Continue(()),
    }
}
