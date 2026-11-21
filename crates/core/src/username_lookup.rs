//! A Meta username lookup, as data a front end can draw.
//!
//! The upstream feature answers in three shapes: an account, a request for the
//! account's username key, or nobody at all. Those are kept distinct here —
//! "needs a key" is not the same answer as "not found", and flattening the two
//! into a missing name would tell somebody to give up when a second ask with a
//! key could resolve it.
//!
//! The verified name is only ever a name the protocol supplied. A handle that
//! happens to look like a business name is not one, and the display name comes
//! from this account's own address book rather than from the handle.

use serde::{Deserialize, Serialize};

/// Shortest and longest handles the server accepts, counting characters.
pub const USERNAME_MIN_LENGTH: usize = 3;
pub const USERNAME_MAX_LENGTH: usize = 35;

/// Strip the display-only `@` and check the handle before it reaches a daemon.
pub fn normalize_username(raw: &str) -> Option<String> {
    let bare = raw.trim().strip_prefix('@').unwrap_or(raw.trim()).trim();
    let length = bare.chars().count();
    (USERNAME_MIN_LENGTH..=USERNAME_MAX_LENGTH)
        .contains(&length)
        .then(|| bare.to_string())
}

/// The account a username resolved to.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ResolvedUsername {
    /// The address the server returned, normally a LID.
    pub jid: String,
    /// The phone-number JID, when the server disclosed one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pn_jid: Option<String>,
    /// The username as the server spelled it back, when it did.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    /// This account's name for the address, when it knows one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// The verified business name from the lookup response, never inferred.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verified_name: Option<String>,
}

/// What a lookup answered.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UsernameLookupResult {
    /// The username resolved to an account.
    Found(ResolvedUsername),
    /// The server withheld the identity: repeat the ask with the account's
    /// username key. The handle is the one the server spelled back, if any.
    KeyRequired {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        username: Option<String>,
    },
    /// No account answers to this handle, or none is reachable from here.
    NotFound,
    /// The handle was rejected before the server was asked.
    InvalidUsername,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_display_handle_is_normalized_before_a_request() {
        assert_eq!(
            normalize_username(" @example.handle ").as_deref(),
            Some("example.handle")
        );
        assert_eq!(
            normalize_username("example.handle").as_deref(),
            Some("example.handle")
        );
    }

    #[test]
    fn handles_outside_the_server_limits_are_refused_locally() {
        assert_eq!(normalize_username("ab"), None);
        assert_eq!(normalize_username("@"), None);
        assert_eq!(
            normalize_username(&"a".repeat(USERNAME_MAX_LENGTH + 1)),
            None
        );
        assert_eq!(
            normalize_username(&"a".repeat(USERNAME_MIN_LENGTH)).as_deref(),
            Some("aaa")
        );
    }

    #[test]
    fn a_found_answer_keeps_only_the_fields_the_server_supplied() {
        let found = UsernameLookupResult::Found(ResolvedUsername {
            jid: "100000000000001@lid".into(),
            pn_jid: None,
            username: Some("example.handle".into()),
            name: None,
            verified_name: None,
        });
        let line = serde_json::to_string(&found).expect("an answer is writable");
        assert_eq!(
            line,
            r#"{"found":{"jid":"100000000000001@lid","username":"example.handle"}}"#
        );
        assert_eq!(
            serde_json::from_str::<UsernameLookupResult>(&line).unwrap(),
            found
        );
    }

    #[test]
    fn an_answer_without_optional_fields_still_deserializes() {
        let line = r#"{"found":{"jid":"15555550100@s.whatsapp.net"}}"#;
        assert_eq!(
            serde_json::from_str::<UsernameLookupResult>(line).unwrap(),
            UsernameLookupResult::Found(ResolvedUsername {
                jid: "15555550100@s.whatsapp.net".into(),
                ..Default::default()
            })
        );
        assert_eq!(
            serde_json::from_str::<UsernameLookupResult>(r#"{"key_required":{}}"#).unwrap(),
            UsernameLookupResult::KeyRequired { username: None }
        );
        for line in [r#""not_found""#, r#""invalid_username""#] {
            let parsed = serde_json::from_str::<UsernameLookupResult>(line).unwrap();
            assert!(matches!(
                parsed,
                UsernameLookupResult::NotFound | UsernameLookupResult::InvalidUsername
            ));
        }
    }
}
