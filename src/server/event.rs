use crate::shared::event::KeyType as ClientKeyType;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "keytype", rename_all = "UPPERCASE")]
pub(crate) enum KeyType {
    Typing,
    Deletion,
    Other,
}

impl From<ClientKeyType> for KeyType {
    fn from(value: ClientKeyType) -> Self {
        match value {
            ClientKeyType::Typing => KeyType::Typing,
            ClientKeyType::Deletion => KeyType::Deletion,
            ClientKeyType::Other => KeyType::Other,
        }
    }
}

