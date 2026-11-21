//! Channel (newsletter) requests and their results.
//!
//! The boundary is deliberately dull: JIDs and message ids are strings, option
//! indices are `u32`, and nothing else crosses it. No server ids, no option
//! hashes and no library group/message reference types live here — the side
//! that owns those converts at its own edge.
//!
//! [`ChannelReact::emoji`] is empty for a removal, not a blank reaction.

use serde::{Deserialize, Serialize};

/// A poll to open in a channel.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChannelPollDraft {
    pub channel_jid: String,
    pub question: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<String>,
    pub selectable_count: u32,
}

/// A reaction to a channel message. An empty `emoji` removes the sender's
/// reaction rather than adding one with no text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChannelReact {
    pub channel_jid: String,
    pub message_id: String,
    pub emoji: String,
}

/// A vote on a channel poll, by the indices of the chosen options.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChannelVotePoll {
    pub channel_jid: String,
    pub poll_message_id: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub option_indices: Vec<u32>,
}

/// The message a channel operation created.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChannelPollCreated {
    pub channel_jid: String,
    pub message_id: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    const CHANNEL: &str = "155555500001@newsletter";

    #[test]
    fn channel_requests_round_trip() {
        let poll = ChannelPollDraft {
            channel_jid: CHANNEL.into(),
            question: "Synthetic question".into(),
            options: vec!["First".into(), "Second".into()],
            selectable_count: 1,
        };
        assert_eq!(
            serde_json::from_value::<ChannelPollDraft>(serde_json::to_value(&poll).unwrap())
                .unwrap(),
            poll
        );

        let react = ChannelReact {
            channel_jid: CHANNEL.into(),
            message_id: "SYNTHETIC-1".into(),
            emoji: "👍".into(),
        };
        assert_eq!(
            serde_json::from_value::<ChannelReact>(serde_json::to_value(&react).unwrap()).unwrap(),
            react
        );

        let vote = ChannelVotePoll {
            channel_jid: CHANNEL.into(),
            poll_message_id: "SYNTHETIC-2".into(),
            option_indices: vec![0, 2],
        };
        assert_eq!(
            serde_json::from_value::<ChannelVotePoll>(serde_json::to_value(&vote).unwrap())
                .unwrap(),
            vote
        );

        let created = ChannelPollCreated {
            channel_jid: CHANNEL.into(),
            message_id: "SYNTHETIC-3".into(),
        };
        assert_eq!(
            serde_json::from_value::<ChannelPollCreated>(serde_json::to_value(&created).unwrap())
                .unwrap(),
            created
        );
    }

    #[test]
    fn missing_collections_default_to_empty() {
        let poll: ChannelPollDraft = serde_json::from_str(
            r#"{"channel_jid":"155555500001@newsletter","question":"Synthetic","selectable_count":1}"#,
        )
        .unwrap();
        assert!(poll.options.is_empty());

        let vote: ChannelVotePoll = serde_json::from_str(
            r#"{"channel_jid":"155555500001@newsletter","poll_message_id":"SYNTHETIC"}"#,
        )
        .unwrap();
        assert!(vote.option_indices.is_empty());
    }

    #[test]
    fn an_empty_emoji_survives_as_a_removal_marker() {
        let react = ChannelReact {
            channel_jid: CHANNEL.into(),
            message_id: "SYNTHETIC-1".into(),
            emoji: String::new(),
        };
        let encoded = serde_json::to_string(&react).unwrap();
        assert!(encoded.contains("\"emoji\":\"\""));
        assert_eq!(
            serde_json::from_str::<ChannelReact>(&encoded).unwrap(),
            react
        );
    }
}
