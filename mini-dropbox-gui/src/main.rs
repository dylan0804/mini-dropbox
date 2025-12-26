use std::{clone, ops::ControlFlow, path::PathBuf, sync::Arc};

use anyhow::{anyhow, Context};
use eframe::CreationContext;
use egui::{vec2, Align2, Vec2};
use egui_toast::{Toast, ToastKind, ToastOptions, Toasts};
use futures_util::{SinkExt, StreamExt};
use iroh_blobs::ticket::BlobTicket;
use names::{Generator, Name};
use rfd::FileDialog;
use serde_json::json;
use tokio::sync::mpsc::{self, Receiver, Sender};
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
    active_users_list: Vec<String>,
    toasts: Toasts,
    files: Vec<PathBuf>,
    selected_file: PathBuf,
    tx: Sender<AppEvent>,
    rx: Receiver<AppEvent>,
    to_ws: Sender<WebSocketMessage>,
}

impl MyApp {
    fn new(_cc: &CreationContext) -> Self {
        let (tx, rx) = mpsc::channel::<AppEvent>(100);
        let (to_ws, from_ui) = mpsc::channel::<WebSocketMessage>(100);

        let toasts = Toasts::new()
            .anchor(Align2::RIGHT_TOP, (-10., 10.))
            .order(egui::Order::Tooltip);

        Self {
            app_state: AppState::OnStartup(Some(from_ui)),
            files: vec![],
            selected_file: PathBuf::new(),
            active_users_list: vec![],
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
                    AppEvent::UpdateActiveUsersList(active_users_list) => {
                        self.active_users_list = active_users_list;
                    }
                    AppEvent::FatalError(e) => {
                        self.show_toast(format!("{e:#}"), ToastKind::Error);
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
                            let ws_stream = connect_async("ws://3.107.184.180:4001/ws")
                                .await
                                .context("WebSocket connection failed")?;

                            let (sender, receiver) = ws_stream.0.split();
                            Ok::<_, anyhow::Error>((sender, receiver))
                        };

                        let iroh_init = async {
                            IrohNode::new()
                                .await
                                .context("Iroh node initialization failed")
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
                                        while let Some(websocket_msg) = from_ui.recv().await {
                                            match websocket_msg {
                                                WebSocketMessage::PrepareFile(abs_path) => {
                                                    let tag = iroh_node
                                                        .store
                                                        .blobs()
                                                        .add_path(abs_path)
                                                        .await
                                                        .unwrap();

                                                    let node_id = iroh_node.endpoint.id();

                                                    let ticket = BlobTicket::new(
                                                        node_id.into(),
                                                        tag.hash,
                                                        tag.format,
                                                    )
                                                    .to_string();

                                                    let json = WebSocketMessage::SendFile(ticket)
                                                        .to_json();

                                                    if let Err(e) = sender
                                                        .send(Message::Text(json.into()))
                                                        .await
                                                        .context("Websocket send failed")
                                                    {
                                                        tx_clone
                                                            .send(AppEvent::FatalError(e))
                                                            .await
                                                            .ok();
                                                    }
                                                }
                                                _ => {
                                                    let json = websocket_msg.to_json();
                                                    if let Err(e) = sender
                                                        .send(Message::Text(json.into()))
                                                        .await
                                                        .context("Websocket send failed")
                                                    {
                                                        tx_clone
                                                            .send(AppEvent::FatalError(e))
                                                            .await
                                                            .ok();
                                                    }
                                                }
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
                    if let Err(e) = self.to_ws.try_send(WebSocketMessage::Register {
                        nickname: self.nickname.clone(),
                    }) {
                        self.tx
                            .try_send(AppEvent::FatalError(
                                anyhow!(e).context("Register send failed"),
                            ))
                            .ok();
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
                                    self.selected_file = p.clone();
                                    if let Err(e) = self.to_ws.try_send(
                                        WebSocketMessage::GetActiveUsersList(self.nickname.clone()),
                                    ) {
                                        self.tx
                                            .try_send(AppEvent::FatalError(anyhow!(e).context(
                                                "Failed to send WebSocket message to pipe",
                                            )))
                                            .ok();
                                    }
                                }
                            });
                        })
                    }

                    if !self.selected_file.to_string_lossy().to_string().is_empty() {
                        println!("sending file");
                        let abs_path = std::path::absolute(&self.selected_file).unwrap();

                        if let Err(e) = self.to_ws.try_send(WebSocketMessage::PrepareFile(abs_path))
                        {
                            self.tx
                                .try_send(AppEvent::FatalError(
                                    anyhow!(e).context("failed to send websocket msg"),
                                ))
                                .ok();
                        }
                        self.selected_file = PathBuf::new();
                    }

                    if !self.active_users_list.is_empty() {
                        self.active_users_list.iter().for_each(|u| {
                            if ui.button(u).clicked() {
                                // get iroh credentials (blob and endpoint)
                                // send name and iroh credentials through websocket
                                // websocket finds the name and get the sender for that name
                                // send the blob and endpoint from the client
                                // download

                                let abs_path = std::path::absolute(&self.selected_file).unwrap();
                                if let Err(e) =
                                    self.to_ws.try_send(WebSocketMessage::PrepareFile(abs_path))
                                {
                                    self.tx
                                        .try_send(AppEvent::FatalError(
                                            anyhow!(e).context("failed to send websocket msg"),
                                        ))
                                        .ok();
                                }
                            }
                        });
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
            .try_send(WebSocketMessage::DisconnectUser(self.nickname.clone()))
        {
            self.tx
                .try_send(AppEvent::FatalError(
                    anyhow!(e).context("Disconnect send failed"),
                ))
                .ok();
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
                    tx.send(AppEvent::FatalError(
                        anyhow!(e).context("Server JSON error"),
                    ))
                    .await
                    .ok();
                }
                WebSocketMessage::ActiveUsersList(active_users_list) => {
                    tx.send(AppEvent::UpdateActiveUsersList(active_users_list))
                        .await
                        .ok();
                }
                WebSocketMessage::ReceiveFile(ticket) => {
                    println!("ticket is {ticket}");
                }
                _ => {}
            },
            Err(e) => {
                tx.send(AppEvent::FatalError(
                    anyhow!(e).context("Message parse failed"),
                ))
                .await
                .ok();
            }
        },
        _ => {}
    }

    ControlFlow::Continue(())
}
