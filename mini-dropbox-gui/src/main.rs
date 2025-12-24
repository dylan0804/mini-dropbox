use std::{clone, ops::ControlFlow, path::PathBuf, sync::Arc};

use anyhow::anyhow;
use eframe::CreationContext;
use egui::{vec2, Align2, Vec2};
use egui_toast::{Toast, ToastKind, ToastOptions, Toasts};
use futures_util::{SinkExt, StreamExt};
use names::{Generator, Name};
use reqwest::Client;
use rfd::FileDialog;
use serde_json::json;
use tokio::sync::mpsc::{self, error::TrySendError, Receiver, Sender};
use tokio_tungstenite::{
    connect_async,
    tungstenite::{Bytes, Message, Utf8Bytes},
};

use crate::{events::AppEvent, iroh_node::IrohNode, message::WebSocketMessage, state::AppState};

mod events;
mod iroh_node;
mod message;
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
    toasts: Toasts,
    files: Vec<PathBuf>,
    tx: Sender<AppEvent>,
    rx: Receiver<AppEvent>,
    to_ws: Sender<String>,
}

impl MyApp {
    fn new(_cc: &CreationContext) -> Self {
        let (tx, rx) = mpsc::channel::<AppEvent>(100);
        let (to_ws, from_ui) = mpsc::channel::<String>(100);

        let toasts = Toasts::new()
            .anchor(Align2::RIGHT_TOP, (-10., 10.))
            .order(egui::Order::Tooltip);

        Self {
            app_state: AppState::OnStartup(Some(from_ui)),
            files: vec![],
            nickname: "".into(),
            to_ws,
            toasts,
            rx,
            tx,
        }
    }
}

impl eframe::App for MyApp {
    fn update(&mut self, ctx: &eframe::egui::Context, _frame: &mut eframe::Frame) {
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
            // event handler
            while let Ok(app_event) = self.rx.try_recv() {
                match app_event {
                    AppEvent::ReadyToPublishUser => self.app_state = AppState::PublishUser,
                    AppEvent::AllSystemsGo => self.app_state = AppState::Ready,
                    AppEvent::RegisterSuccess => {
                        self.show_toast("Register success!", ToastKind::Success);
                        self.app_state = AppState::Ready;
                    }
                    AppEvent::FatalError(e) => {
                        self.show_toast(e, ToastKind::Error);
                    }
                }
            }

            match &mut self.app_state {
                AppState::OnStartup(from_ui) => {
                    let mut from_ui = from_ui.take().unwrap();

                    // get nickname
                    let mut generator = Generator::with_naming(Name::Numbered);
                    self.nickname = generator.next().unwrap_or("Guest".into());

                    // setup ws
                    let tx = self.tx.clone();

                    tokio::spawn(async move {
                        let ws_init = async {
                            let ws_stream = match connect_async("ws://3.107.184.180:4001/ws").await
                            {
                                Ok((stream, _)) => stream,
                                Err(e) => {
                                    return Err(e.to_string());
                                }
                            };

                            let (sender, receiver) = ws_stream.split();
                            Ok((sender, receiver))
                        };

                        let iroh_init = async {
                            match IrohNode::new().await {
                                Ok(iroh_node) => Ok(iroh_node),
                                Err(e) => Err(e.to_string()),
                            }
                        };

                        tokio::spawn(async move {
                            match tokio::try_join!(ws_init, iroh_init) {
                                Ok(((mut sender, mut receiver), iroh_node)) => {
                                    // get ws msg
                                    let tx_clone = tx.clone();
                                    tokio::spawn(async move {
                                        loop {
                                            if let Some(Ok(msg)) = receiver.next().await {
                                                if process_message(msg, tx_clone.clone())
                                                    .await
                                                    .is_break()
                                                {
                                                    break;
                                                }
                                            }
                                        }
                                    });

                                    // send ws msg
                                    let tx_clone = tx.clone();
                                    tokio::spawn(async move {
                                        while let Some(msg) = from_ui.recv().await {
                                            if let Err(e) = sender
                                                .send(Message::Text(Utf8Bytes::from(msg)))
                                                .await
                                            {
                                                tx_clone
                                                    .send(AppEvent::FatalError(e.to_string()))
                                                    .await
                                                    .ok();
                                            }
                                        }
                                    });

                                    tx.send(AppEvent::ReadyToPublishUser).await.ok();
                                }
                                Err(e) => {
                                    tx.send(AppEvent::FatalError(e)).await.ok();
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
                AppState::PublishUser => {
                    let json = json!(WebSocketMessage::Register {
                        nickname: self.nickname.clone()
                    })
                    .to_string();
                    if let Err(e) = self.to_ws.try_send(json) {
                        self.tx.try_send(AppEvent::FatalError(e.to_string())).ok();
                    }

                    self.app_state = AppState::WaitForRegisterConfirmation;
                }
                AppState::WaitForRegisterConfirmation => {} // do nothing
                AppState::Ready => {
                    if ui.button("Select file").clicked() {
                        let files = FileDialog::new().set_directory("/").pick_file().unwrap();
                        self.files.push(files);
                    }

                    if !self.files.is_empty() {
                        self.files.iter().for_each(|p| {
                            ui.horizontal(|ui| {
                                ui.label(p.file_name().unwrap().to_string_lossy());
                                if ui.button("Send to").clicked() {
                                    self.to_ws.try_send("Bitch".into()).ok();
                                }
                            });
                        })
                    }
                }
                _ => {}
            }

            self.toasts.show(ctx);
        });
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        if let Err(e) = self
            .to_ws
            .try_send(json!(WebSocketMessage::DisconnectUser(self.nickname.clone())).to_string())
        {
            self.tx.try_send(AppEvent::FatalError(e.to_string())).ok();
        }
    }
}

async fn process_message(msg: Message, tx: Sender<AppEvent>) -> ControlFlow<(), ()> {
    match msg {
        Message::Text(bytes) => match serde_json::from_str::<WebSocketMessage>(bytes.as_str()) {
            Ok(websocket_msg) => match websocket_msg {
                WebSocketMessage::RegisterSuccess => {
                    tx.send(AppEvent::RegisterSuccess).await.ok();
                }
                WebSocketMessage::ErrorDeserializingJson(e) => {
                    tx.send(AppEvent::FatalError(e.to_string())).await.ok();
                }
                _ => {}
            },
            Err(e) => {
                tx.send(AppEvent::FatalError(e.to_string())).await.ok();
            }
        },
        _ => {}
    }
    ControlFlow::Continue(())
}
