use poem_openapi::{OpenApi, OpenApiService};

pub mod admin_login;
pub mod admin_system;
pub mod admin_user;
pub mod archive;
pub mod bot;
pub mod matrix;
pub mod datetime;
pub mod favorite;
pub mod group;
pub mod langid;
pub use matrix::create_api_routes;
pub use matrix::init_bot_cache;
pub mod message;
pub mod message_api;
pub mod resource;
pub mod tags;
pub mod token;
pub mod user;
pub mod user_log_action;

pub use admin_login::LoginConfig;
pub use admin_system::{FrontendUrlConfig, OrganizationConfig};
pub use admin_user::{User, UserDevice};
pub use archive::Archive;
pub use datetime::DateTime;
pub use group::{Group, GroupAnnouncement, PinnedMessage};
pub use langid::LangId;
pub use message::{
    get_merged_message, BurnAfterReadingGroup, BurnAfterReadingUser, ChatMessage,
    ChatMessagePayload, GroupChangedMessage, HeartbeatMessage, KickFromGroupReason, KickMessage,
    KickReason, Message, MessageDetail, MessageTarget, MessageTargetGroup, MessageTargetUser,
    MuteGroup, MuteUser, PinChat, PinChatTarget, PinChatTargetChannel, PinChatTargetUser,
    ReadIndexGroup, ReadIndexUser, UserSettingsChangedMessage, UserSettingsMessage,
    UserStateChangedMessage, UserUpdateLog, UsersUpdateLogMessage,
};
pub use resource::FileMeta;
pub use token::{CurrentUser, Token};
pub use user::{
    CreateUserConflictReason, CreateUserResponse, UpdateUserResponse, UserConflict, UserInfo,
};
pub use user_log_action::UpdateAction;

pub fn create_api_service() -> OpenApiService<impl OpenApi, ()> {
    OpenApiService::new(
        (
            token::ApiToken,
            user::ApiUser,
            group::ApiGroup,
            admin_user::ApiAdminUser,
            resource::ApiResource,
            message_api::ApiMessage,
            favorite::ApiFavorite,
            admin_system::ApiAdminSystem,
            admin_login::ApiAdminLogin,
            bot::ApiBot,
        ),
        "vachat",
        env!("CARGO_PKG_VERSION"),
    )
}
