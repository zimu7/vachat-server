//! E2EE data types for Matrix protocol

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Device keys structure as per Matrix spec
/// https://spec.matrix.org/v1.11/client-server-api/#key-format
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceKeys {
    /// User ID
    pub user_id: String,
    /// Device ID
    pub device_id: String,
    /// List of algorithms supported
    pub algorithms: Vec<String>,
    /// Map of key ID to base64 encoded key
    pub keys: std::collections::HashMap<String, String>,
    /// Signatures of the keys
    pub signatures: std::collections::HashMap<String, std::collections::HashMap<String, String>>,
    /// Optional: User-friendly device name
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device_display_name: Option<String>,
    /// Optional: Additional data
    #[serde(flatten)]
    pub extra: std::collections::HashMap<String, Value>,
}

/// One-time key structure
/// https://spec.matrix.org/v1.11/client-server-api/#post_matrixclientv3keysupload
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OneTimeKey {
    /// Base64 encoded Curve25519 key
    pub key: String,
    /// Optional signatures object
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signatures: Option<std::collections::HashMap<String, std::collections::HashMap<String, String>>>,
}

/// Keys upload request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeysUploadRequest {
    /// Device keys to upload
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device_keys: Option<DeviceKeys>,
    /// One-time keys to upload
    /// Supports both formats:
    /// 1. Nested: {"signed_curve25519": {"key_id": {...}}}
    /// 2. Flat: {"signed_curve25519:key_id": {...}}
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub one_time_keys: Option<std::collections::HashMap<String, serde_json::Value>>,
    /// Fallback keys (not implemented)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fallback_keys: Option<std::collections::HashMap<String, OneTimeKey>>,
}
