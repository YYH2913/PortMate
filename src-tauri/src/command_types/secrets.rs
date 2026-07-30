use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SecretWriteRequest {
    pub secret_ref: Option<String>,
    pub secret: String,
    pub storage: Option<SecretStorage>,
}

#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "kebab-case")]
pub enum ProxyPasswordUpdate {
    Set {
        password: String,
        #[serde(default)]
        storage: Option<SecretStorage>,
    },
    Clear,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SecretStorage {
    Native,
    Portable,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SecretWriteResponse {
    pub secret_ref: String,
}
