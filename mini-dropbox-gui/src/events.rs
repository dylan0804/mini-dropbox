use anyhow::Error;

pub enum AppEvent {
    WebSocketSuccess,
    WebSocketFailed(Error),
}
