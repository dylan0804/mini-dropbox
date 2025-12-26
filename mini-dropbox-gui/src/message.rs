use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
#[serde(tag = "type", content = "payload", rename_all = "snake_case")]
pub enum WebSocketMessage {
    Register { nickname: String },
    DisconnectUser(String),

    RegisterSuccess,

    GetActiveUsersList(String),
    ActiveUsersList(Vec<String>),

    PrepareFile(PathBuf),
    SendFile { recipient: String, ticket: String },
    ReceiveFile(String),

    ErrorDeserializingJson(String),
    UserNotFound,
}

impl WebSocketMessage {
    pub fn to_json(&self) -> String {
        match self {
            WebSocketMessage::Register { nickname } => {
                serde_json::to_string(&WebSocketMessage::Register {
                    nickname: nickname.clone(),
                })
                .unwrap()
            }
            WebSocketMessage::DisconnectUser(nickname) => {
                serde_json::to_string(&WebSocketMessage::DisconnectUser(nickname.clone())).unwrap()
            }
            WebSocketMessage::RegisterSuccess => {
                serde_json::to_string(&WebSocketMessage::RegisterSuccess).unwrap()
            }
            WebSocketMessage::GetActiveUsersList(except) => {
                serde_json::to_string(&WebSocketMessage::GetActiveUsersList(except.clone()))
                    .unwrap()
            }
            WebSocketMessage::SendFile { recipient, ticket } => {
                serde_json::to_string(&WebSocketMessage::SendFile {
                    recipient: recipient.clone(),
                    ticket: ticket.clone(),
                })
                .unwrap()
            }
            _ => "".into(),
        }
    }
}
