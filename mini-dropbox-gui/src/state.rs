use tokio::sync::mpsc::Receiver;

use crate::message::WebSocketMessage;

#[derive(Debug)]
pub enum AppState {
    OnStartup(Option<Receiver<WebSocketMessage>>),
    Connecting,
    Ready,

    WebSocketReady,
    IrohReady,

    PublishUser,
    WaitForRegisterConfirmation,
}
