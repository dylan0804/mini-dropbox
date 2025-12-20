#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppState {
    OnStartup,
    Connecting,
    Ready,
}
