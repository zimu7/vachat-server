//! Matrix API module
//!
//! This module contains all Matrix protocol endpoints organized into submodules:
//! - auth: Authentication and login
//! - sync: Message sync and long polling
//! - rooms: Room-related endpoints
//! - account: User account endpoints
//! - keys: E2EE keys management
//! - e2ee: End-to-End encryption types and managers
//! - to_device: To-device message handling (room key distribution)
//! - media: Media download endpoints

mod account;
pub(crate) mod auth;
pub mod e2ee;
mod keys;
mod media;
mod rooms;
mod sync;
mod to_device;

pub use sync::{init_bot_cache, invalidate_bot_cache};

use poem::Route;

/// Create poem native routes for Matrix endpoints
pub fn create_api_routes() -> Route {
    Route::new()
        .at("/client/v3/login", auth::login)
        .at("/client/v3/sync", sync::sync)
        .at("/client/v3/rooms/**", rooms::rooms_handler)
        .at("/client/v3/account/whoami", account::whoami)
        .at("/client/v3/user/**", account::user_handler)
        .at("/client/v3/pushrules", account::pushrules)
        .at("/client/v3/pushrules/**", account::pushrules)
        .at("/client/v3/capabilities", account::capabilities)
        .at("/client/v3/keys/query", keys::keys_query)
        .at("/client/v3/keys/upload", keys::keys_upload)
        .at("/client/v3/keys/claim", keys::keys_claim)
        .at("/client/v3/sendToDevice/**", to_device::send_to_device)
        .at("/client/v3/devices**", account::devices_handler)
        .at("/client/versions", account::versions)
        // Media download endpoint
        .at("/media/v3/download/:server/:media_id", media::download)
        // Also support /_matrix/media/v3/download/... path with filename
        .at("/media/v3/download/:server/:media_id/**", media::download_with_filename)
        // Matrix client-v1 media download endpoint (used by Hermes)
        .at("/client/v1/media/download/:server/:media_id", media::download)
        .at("/client/v1/media/download/:server/:media_id/**", media::download_with_filename)
}
