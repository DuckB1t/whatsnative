//! Who this account has blocked.
//!
//! The server owns this list: it is not derivable from chats or contacts, and
//! it is not state a window can reconstruct from a snapshot. The session reads
//! it on request, names whoever it can from the local address book, and this
//! is the shape that travels to a front end.
//!
//! The timestamp is the one the server sent, in Unix seconds — not the
//! millisecond clock the rest of the UI uses. It is optional because older
//! entries on the wire carry no time at all, and a made-up one would be worse
//! than none.

use serde::{Deserialize, Serialize};
use wacore_binary::jid::Jid;

/// One address on the account's block list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlockedContact {
    /// The address as the server wrote it: a PN or a LID, never a group.
    pub jid: String,
    /// The address book's name, when this account knows one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Unix seconds, when the server supplied a time.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blocked_at: Option<u64>,
}

impl BlockedContact {
    /// What a row shows: a name when there is one, the address otherwise.
    ///
    /// The JID stays with the entry rather than being replaced by its name:
    /// the Unblock button has to send the exact address the server gave back,
    /// including its namespace.
    #[must_use]
    pub fn label(&self) -> &str {
        self.name.as_deref().unwrap_or(&self.jid)
    }
}

/// Normalize what somebody typed into the Add field into a blockable address.
///
/// A bare number is treated as a PN, while an explicit `user@server` keeps
/// the namespace it was given. Both are stripped to the non-device address:
/// a device JID addresses one phone, and the blocklist addresses a person.
/// Groups, broadcasts, channels and anything that does not parse are refused
/// here, before a request reaches the daemon — the server would reject them,
/// and the person who typed them is owed a reason rather than a failed IQ.
pub fn normalize_block_target(raw: &str) -> Result<String, String> {
    let typed = raw.trim();
    if typed.is_empty() {
        return Err("enter a phone number or JID".to_string());
    }
    let candidate = if typed.contains('@') {
        typed.to_string()
    } else {
        format!("{typed}@s.whatsapp.net")
    };
    let jid: Jid = candidate
        .parse()
        .map_err(|_| "that is not a phone number or JID".to_string())?;
    let bare = jid.into_non_ad();
    let user = bare.user_base();
    // The JID parser accepts any string before `@`, including spaces and
    // punctuation. A block target is a person's address, which is digits in
    // either namespace — reject anything else here rather than sending a
    // request the server can only refuse.
    if !(bare.server.is_pn_family() || bare.server.is_lid_family())
        || user.is_empty()
        || !user.bytes().all(|c| c.is_ascii_digit())
    {
        return Err("only a phone number or a user JID can be blocked".to_string());
    }
    Ok(bare.to_non_ad_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_nameless_entry_reads_as_its_address() {
        let entry = BlockedContact {
            jid: "15555550100@s.whatsapp.net".into(),
            name: None,
            blocked_at: None,
        };
        assert_eq!(entry.label(), entry.jid);
    }

    #[test]
    fn optional_fields_are_absent_from_the_wire() {
        let entry = BlockedContact {
            jid: "100000000000001@lid".into(),
            name: None,
            blocked_at: None,
        };
        let line = serde_json::to_string(&entry).expect("an entry is writable");
        assert_eq!(line, r#"{"jid":"100000000000001@lid"}"#);
        assert_eq!(
            serde_json::from_str::<BlockedContact>(&line).unwrap(),
            entry
        );
    }

    #[test]
    fn a_bare_number_becomes_a_phone_number_jid() {
        assert_eq!(
            normalize_block_target(" 15555550100 ").unwrap(),
            "15555550100@s.whatsapp.net"
        );
    }

    #[test]
    fn an_explicit_namespace_is_kept_and_devices_are_stripped() {
        assert_eq!(
            normalize_block_target("100000000000001:7@lid").unwrap(),
            "100000000000001@lid"
        );
        assert_eq!(
            normalize_block_target("15555550100@s.whatsapp.net").unwrap(),
            "15555550100@s.whatsapp.net"
        );
    }

    #[test]
    fn non_user_targets_are_refused() {
        for typed in [
            "",
            "   ",
            "not a number",
            "120363000000000001@g.us",
            "status@broadcast",
            "123@newsletter",
        ] {
            assert!(
                normalize_block_target(typed).is_err(),
                "{typed:?} should not be blockable"
            );
        }
    }
}
