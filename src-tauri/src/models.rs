use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Group {
    pub id: i64,
    pub name: String,
    pub parent_id: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Host {
    pub id: i64,
    pub name: String,
    pub address: String,
    pub port: u16,
    pub username: String,
    pub group_id: Option<i64>,
    pub auth_type: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Library {
    pub groups: Vec<Group>,
    pub hosts: Vec<Host>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GroupInput {
    pub id: Option<i64>,
    pub name: String,
    pub parent_id: Option<i64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HostInput {
    pub id: Option<i64>,
    pub name: String,
    pub address: String,
    pub port: u16,
    pub username: String,
    pub group_id: Option<i64>,
    pub auth_type: String,
    pub password: Option<String>,
    pub private_key_path: Option<String>,
    pub private_key_passphrase: Option<String>,
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct TerminalOutput {
    pub session_id: String,
    pub data: Vec<u8>,
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct TerminalStatus {
    pub session_id: String,
    pub status: String,
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Credential {
    Password {
        password: String,
    },
    PrivateKey {
        private_key: String,
        passphrase: Option<String>,
    },
}

#[derive(Debug, Serialize, Deserialize)]
pub struct BackupData {
    pub version: u32,
    pub groups: Vec<BackupGroup>,
    pub hosts: Vec<BackupHost>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct BackupGroup {
    pub id: i64,
    pub name: String,
    pub parent_id: Option<i64>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct BackupHost {
    pub name: String,
    pub address: String,
    pub port: u16,
    pub username: String,
    pub group_id: Option<i64>,
    pub auth_type: String,
    pub credential: Credential,
}
