use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
#[serde(tag = "type", content = "payload", rename_all = "snake_case")]
pub enum WebSocketMessage {
    Register { nickname: String },
    DisconnectUser(String),

    RegisterSuccess,

    ErrorDeserializingJson(String),
}
