use md5::{Md5, Digest};

/// Hash password with salt using MD5
/// Format: md5(md5(password) + salt)
pub fn hash_password(password: &str, salt: &str) -> String {
    let mut hasher = Md5::new();
    hasher.update(password.as_bytes());
    let first_hash = hasher.finalize();
    let first_hash_hex = hex::encode(first_hash);

    let mut hasher = Md5::new();
    hasher.update(format!("{}{}", first_hash_hex, salt).as_bytes());
    let final_hash = hasher.finalize();
    hex::encode(final_hash)
}

/// Verify password against stored hash
pub fn verify_password(password: &str, salt: &str, stored_hash: &str) -> bool {
    let computed_hash = hash_password(password, salt);
    computed_hash == stored_hash
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hash_password() {
        let password = "123456";
        let salt = "test_salt";
        let hash = hash_password(password, salt);
        assert!(!hash.is_empty());
        assert_eq!(hash.len(), 32); // MD5 produces 32 hex chars
    }

    #[test]
    fn test_verify_password() {
        let password = "123456";
        let salt = "test_salt";
        let hash = hash_password(password, salt);
        assert!(verify_password(password, salt, &hash));
        assert!(!verify_password("wrong_password", salt, &hash));
    }

    #[test]
    fn test_same_password_different_salts() {
        let password = "123456";
        let hash1 = hash_password(password, "salt1");
        let hash2 = hash_password(password, "salt2");
        assert_ne!(hash1, hash2);
    }
}
