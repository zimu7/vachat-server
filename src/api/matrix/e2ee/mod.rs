//! E2EE (End-to-End Encryption) module for Matrix protocol
//!
//! This module provides:
//! - Device keys management (Curve25519, Ed25519)
//! - One-time pre-keys (OTK) for Olm session establishment
//! - Room encryption state management
//! - Olm session management for DM encryption
//! - Megolm session management for room/group encryption
//! - Server-side Olm Account for bot virtual devices

mod device_keys;
mod megolm_session;
mod olm_session;
mod room_encryption;
mod server_olm_account;
mod types;

pub use device_keys::DeviceKeysManager;
pub use megolm_session::MegolmSessionManager;
pub use olm_session::OlmSessionManager;
pub use room_encryption::RoomEncryptionManager;
pub use server_olm_account::{ServerOlmAccountManager, SERVER_DEVICE_ID};
pub use types::*;
