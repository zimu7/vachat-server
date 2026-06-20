use poem_openapi::Tags;

#[derive(Tags)]
pub enum ApiTags {
    /// Token operations
    Token,

    /// User operations
    User,

    /// Group operations
    Group,

    /// Message operations
    Message,

    /// Resource operations
    Resource,

    /// Favorite archive operations
    Favorite,

    /// User management operations
    AdminUser,

    /// System management operations
    AdminSystem,

    /// Login management operations
    AdminLogin,

    /// Bot operations
    Bot
}
