//! Secure Boot: approving the DKMS module-signing key ("MOK").
//!
//! With Secure Boot on, the kernel only loads modules signed by a trusted key.
//! DKMS signs the OpenFuse module with a machine-local key. The helper
//! (`openfuse-setup enroll`) queues that key with a password, and on the next
//! boot the firmware's MokManager (a blue screen) asks the person at the
//! computer to type it, which approves the key.
//!
//! The password only proves someone is physically present to approve the key.
//! It defaults to the public "openfuse" (nothing to invent or remember); the
//! user can choose their own on the key page. Trade-off of the default:
//! software that already has root could queue its own key with the same
//! password, and a user expecting "openfuse" might approve it. Such software
//! already controls the system.

/// Typed once at the MokManager screen. All lowercase letters that sit on the
/// same keys in QWERTY, QWERTZ and AZERTY layouts (MokManager uses US layout).
pub const MOK_PASSWORD: &str = "openfuse";

/// What the helper and mokutil accept: 1-256 printable ASCII characters.
pub fn valid_password(p: &str) -> bool {
    (1..=256).contains(&p.len()) && p.chars().all(|c| (' '..='~').contains(&c))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn own_passwords_are_checked_like_the_helper_does() {
        assert!(valid_password("openfuse") && valid_password("My pass 123!"));
        assert!(!valid_password("") && !valid_password("héllo") && !valid_password(&"x".repeat(257)));
    }

    #[test]
    fn password_is_layout_safe() {
        // Letters that move between QWERTY, QWERTZ and AZERTY: a, q, w, y, z, m.
        assert!(MOK_PASSWORD.chars().all(|c| c.is_ascii_lowercase() && !"aqwyzm".contains(c)));
        // mokutil accepts 1-256 printable ASCII characters.
        assert!((1..=256).contains(&MOK_PASSWORD.len()));
    }
}
