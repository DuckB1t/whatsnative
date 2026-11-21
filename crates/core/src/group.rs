//! Who is in a group.
//!
//! Separate from [`Chat::participants`](crate::Chat::participants), which is
//! not a roster and never was: that map is filled as senders are *seen*, so a
//! fifty-person group with one recent sender has one entry in it. This is the
//! membership list itself, as the account's own connection reports it, and it
//! is the only thing that may be counted or listed as "who is in here".

use serde::{Deserialize, Serialize};

/// Whether a JID addresses a WhatsApp group rather than a user, broadcast
/// list or newsletter.
///
/// Parsing is deliberately strict: an address that cannot be parsed is not a
/// call target at all, so it must never be sent down the group path just
/// because it happens to end in `@g.us`. One helper for every front end —
/// the toolbar gate, the optimistic outgoing card and the request sent to the
/// daemon must all agree on what a group is.
#[must_use]
pub fn is_group_jid(jid: &str) -> bool {
    jid.parse::<wacore_binary::jid::Jid>().is_ok_and(|parsed| {
        use wacore_binary::jid::JidExt as _;
        parsed.is_group()
    })
}

/// The members of one group.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GroupRoster {
    /// The group this describes.
    pub jid: String,
    /// Everyone in it, in the order the connection listed them, with this
    /// account's own entry marked rather than moved: where "You" goes in a
    /// line of names is the renderer's decision, not this type's.
    pub members: Vec<GroupMember>,
}

/// One member of a group.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GroupMember {
    /// How the group addresses them, which in a LID-addressed group is a LID
    /// and not a number.
    pub jid: String,
    /// What to call them, or `None` when nobody has ever said.
    ///
    /// Absent rather than filled with the number, for the reason
    /// [`ChatMessage::sender_name`](crate::ChatMessage::sender_name) is:
    /// drawing a stranger is the renderer's job, and a field that has already
    /// been given a number can never be improved by a name arriving later.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Whether this member is the account reading it.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub is_self: bool,
}

impl GroupRoster {
    /// How many people are in the group.
    #[must_use]
    pub fn size(&self) -> usize {
        self.members.len()
    }
}

#[cfg(test)]
mod tests {
    use super::is_group_jid;
    use crate::CALL_LINK_RECIPIENT_JID;

    #[test]
    fn only_a_parsed_group_address_is_a_group() {
        assert!(is_group_jid("120363012345678901@g.us"));
        assert!(!is_group_jid("5521999999999@s.whatsapp.net"));
        assert!(!is_group_jid("5521999999999@lid"));
        assert!(!is_group_jid("status@broadcast"));
        assert!(!is_group_jid("newsletter@g.us@invalid"));
        assert!(!is_group_jid("not a jid"));
    }

    #[test]
    fn is_group_jid_rejects_call_link_tokens() {
        assert!(!is_group_jid("TEST-CALL-LINK"));
        assert!(!is_group_jid("TEST-4821-LINK-9021"));
        assert!(!is_group_jid(CALL_LINK_RECIPIENT_JID));
    }
}
