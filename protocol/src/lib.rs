//! Versioned native transport contract. No account credentials belong in this protocol.
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
pub const VERSION: u16 = 1;
pub const CHUNK_BYTES: usize = 64 * 1024;
pub const MAX_BODY_BYTES: usize = 50 * 1024 * 1024;
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Request {
    pub version: u16,
    pub method: String,
    pub path: String,
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResponseHead {
    pub status: u16,
    pub headers: BTreeMap<String, String>,
    pub stream_id: String,
}
