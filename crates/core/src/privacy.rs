//! Privacy categories this front end can read and set, plus the dedicated
//! three-state DTO for account-level direct-message read receipts.
//!
//! The generic DTOs name a category and a value using the upstream wire names,
//! and keep an Unknown state for answers this build does not understand. The
//! setter gate in [`PrivacySettingCategory::accepts`] refuses
//! `contact_blacklist`: changing it needs a separate disallowed-list update
//! this wave does not offer, so recognising the value is not permission to
//! send it.
//!
//! The server's answer is a list of categories, and `readreceipts` is one of
//! them. It has two documented values — `all` and `none` — and this DTO keeps a
//! third state for a category that was absent or carried a value this build
//! does not know. Coercing either of those to "on" would draw a switch over a
//! fact the server never told us, and a device whose account changed setting
//! elsewhere would be told the opposite of the truth.
//!
//! This covers DM read and played receipts only. Status read receipts, group
//! receipts, story audience, last seen and About visibility are separate
//! account settings and are not described by this type.

use serde::{Deserialize, Serialize};

/// What the account currently does with DM read receipts, as the server said.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PrivacyReadReceipts {
    /// The server answered `all`: DM read receipts are enabled.
    All,
    /// The server answered `none`: DM read receipts are disabled.
    None,
    /// The category was missing, or its wire value was not `all` or `none`.
    ///
    /// A value from a newer server is neither an error nor a synonym for
    /// `all`; the honest answer is that this build cannot read it.
    #[default]
    #[serde(other)]
    Unknown,
}

impl PrivacyReadReceipts {
    /// Map one wire value, keeping anything unrecognised as [`Self::Unknown`].
    #[must_use]
    pub fn from_wire(value: &str) -> Self {
        match value {
            "all" => Self::All,
            "none" => Self::None,
            _ => Self::Unknown,
        }
    }

    /// Whether this is a value the server actually stated.
    ///
    /// The pane draws a switch only for one of the two known states; a
    /// missing or unrecognised answer gets a sentence instead of a control.
    #[must_use]
    pub fn is_known(self) -> bool {
        matches!(self, Self::All | Self::None)
    }
}

/// One privacy category the front end can read, and may be able to set.
///
/// The serde names are the DTO names used over the socket. Upstream uses a
/// shorter set of names on its own wire, which [`Self::from_wire`] and
/// [`Self::to_wire`] translate. The last variant is what an answer this build
/// cannot name becomes, rather than a guess at a known category.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PrivacySettingCategory {
    /// Who can see the account's last seen timestamp. Wire name `last`.
    LastSeen,
    /// Whether the account appears online. Wire name `online`.
    Online,
    /// Who can see the profile photo. Wire name `profile`.
    ProfilePhoto,
    /// Who can see the About text. Wire name `status`.
    About,
    /// Who can add this account to a group. Wire name `groupadd`.
    GroupAdd,
    /// Who can message this account. Wire name `messages`.
    Messages,
    /// Account-level DM read receipts. Wire name `readreceipts`.
    ReadReceipts,
    /// A category this build does not know.
    #[default]
    #[serde(other)]
    Unknown,
}

impl PrivacySettingCategory {
    /// Map one upstream category name, keeping anything unrecognised as
    /// [`Self::Unknown`].
    #[must_use]
    pub fn from_wire(name: &str) -> Self {
        match name {
            "last" => Self::LastSeen,
            "online" => Self::Online,
            "profile" => Self::ProfilePhoto,
            "status" => Self::About,
            "groupadd" => Self::GroupAdd,
            "messages" => Self::Messages,
            "readreceipts" => Self::ReadReceipts,
            _ => Self::Unknown,
        }
    }

    /// The upstream category name, or `None` when this is [`Self::Unknown`].
    #[must_use]
    pub fn to_wire(self) -> Option<&'static str> {
        match self {
            Self::LastSeen => Some("last"),
            Self::Online => Some("online"),
            Self::ProfilePhoto => Some("profile"),
            Self::About => Some("status"),
            Self::GroupAdd => Some("groupadd"),
            Self::Messages => Some("messages"),
            Self::ReadReceipts => Some("readreceipts"),
            Self::Unknown => None,
        }
    }

    /// Whether this category can be set to `value` with the plain setter.
    ///
    /// [`PrivacySettingValue::ContactBlacklist`] is recognised so a published
    /// answer can be read, but it is never accepted here: setting it needs a
    /// separate disallowed-list update this wave does not offer. An
    /// [`Self::Unknown`] category accepts nothing, because this build cannot
    /// know which values the server would permit.
    #[must_use]
    pub fn accepts(self, value: PrivacySettingValue) -> bool {
        match self {
            Self::LastSeen | Self::ProfilePhoto | Self::About | Self::GroupAdd => {
                matches!(
                    value,
                    PrivacySettingValue::All
                        | PrivacySettingValue::Contacts
                        | PrivacySettingValue::None
                )
            }
            Self::Online => matches!(
                value,
                PrivacySettingValue::All | PrivacySettingValue::MatchLastSeen
            ),
            Self::Messages => matches!(
                value,
                PrivacySettingValue::All | PrivacySettingValue::Contacts
            ),
            Self::ReadReceipts => {
                matches!(value, PrivacySettingValue::All | PrivacySettingValue::None)
            }
            Self::Unknown => false,
        }
    }
}

/// One value a privacy category can carry, as read from or sent to upstream.
///
/// The serde names are the DTO names used over the socket. Upstream spells some
/// of them differently, which [`Self::from_wire`] and [`Self::to_wire`]
/// translate. Recognising a value does not make it settable through every
/// category; see [`PrivacySettingCategory::accepts`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PrivacySettingValue {
    /// Everyone. Wire name `all`.
    All,
    /// Only contacts. Wire name `contacts`.
    Contacts,
    /// Nobody. Wire name `none`.
    None,
    /// Follow the last seen setting. Wire name `match_last_seen`.
    MatchLastSeen,
    /// A named disallowed list. Wire name `contact_blacklist`.
    ///
    /// Read but never offered by the plain setter: the list itself has to be
    /// updated separately, which this wave does not do.
    ContactBlacklist,
    /// A value this build does not know.
    #[default]
    #[serde(other)]
    Unknown,
}

impl PrivacySettingValue {
    /// Map one upstream value, keeping anything unrecognised as
    /// [`Self::Unknown`].
    #[must_use]
    pub fn from_wire(value: &str) -> Self {
        match value {
            "all" => Self::All,
            "contacts" => Self::Contacts,
            "none" => Self::None,
            "match_last_seen" => Self::MatchLastSeen,
            "contact_blacklist" => Self::ContactBlacklist,
            _ => Self::Unknown,
        }
    }

    /// The upstream value, or `None` when this is [`Self::Unknown`].
    #[must_use]
    pub fn to_wire(self) -> Option<&'static str> {
        match self {
            Self::All => Some("all"),
            Self::Contacts => Some("contacts"),
            Self::None => Some("none"),
            Self::MatchLastSeen => Some("match_last_seen"),
            Self::ContactBlacklist => Some("contact_blacklist"),
            Self::Unknown => None,
        }
    }

    /// Whether the plain setter can send this value for a category that
    /// accepts it. [`Self::ContactBlacklist`] needs a separate disallowed-list
    /// update and [`Self::Unknown`] is never sent.
    #[must_use]
    pub fn is_settable(self) -> bool {
        matches!(
            self,
            Self::All | Self::Contacts | Self::None | Self::MatchLastSeen
        )
    }
}

/// One category and the value the server published for it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrivacySetting {
    pub category: PrivacySettingCategory,
    pub value: PrivacySettingValue,
}

/// The server's answer for the privacy categories this front end understands.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrivacySettings {
    pub settings: Vec<PrivacySetting>,
}

impl PrivacySettings {
    /// The published setting for `category`, if the answer carried one.
    #[must_use]
    pub fn get(&self, category: PrivacySettingCategory) -> Option<&PrivacySetting> {
        self.settings
            .iter()
            .find(|setting| setting.category == category)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_two_documented_values_map_to_their_states() {
        assert_eq!(
            PrivacyReadReceipts::from_wire("all"),
            PrivacyReadReceipts::All
        );
        assert_eq!(
            PrivacyReadReceipts::from_wire("none"),
            PrivacyReadReceipts::None
        );
        assert!(PrivacyReadReceipts::All.is_known());
        assert!(PrivacyReadReceipts::None.is_known());
    }

    #[test]
    fn a_missing_or_unexpected_value_stays_unknown() {
        for value in ["", "contacts", "ALL", "on", "off", "all "] {
            assert_eq!(
                PrivacyReadReceipts::from_wire(value),
                PrivacyReadReceipts::Unknown,
                "{value:?} is not a read-receipts value"
            );
        }
        assert!(!PrivacyReadReceipts::Unknown.is_known());
    }

    #[test]
    fn the_dto_has_a_three_way_wire_shape() {
        for (value, expected) in [
            (r#""all""#, PrivacyReadReceipts::All),
            (r#""none""#, PrivacyReadReceipts::None),
            (r#""contact_blacklist""#, PrivacyReadReceipts::Unknown),
            (r#""unknown""#, PrivacyReadReceipts::Unknown),
        ] {
            let decoded: PrivacyReadReceipts =
                serde_json::from_str(value).expect("a read-receipts value decodes");
            assert_eq!(decoded, expected, "{value} changed meaning on the wire");
        }
        assert_eq!(
            serde_json::to_string(&PrivacyReadReceipts::None).unwrap(),
            r#""none""#
        );
        // A field a legacy frame left out is unknown, never on.
        #[derive(Deserialize)]
        struct Frame {
            #[serde(default)]
            read_receipts: PrivacyReadReceipts,
        }
        let frame: Frame = serde_json::from_str("{}").unwrap();
        assert_eq!(frame.read_receipts, PrivacyReadReceipts::Unknown);
    }

    #[test]
    fn every_upstream_category_name_maps_to_its_dto_and_back() {
        for (wire, dto_name, expected) in [
            ("last", "last_seen", PrivacySettingCategory::LastSeen),
            ("online", "online", PrivacySettingCategory::Online),
            (
                "profile",
                "profile_photo",
                PrivacySettingCategory::ProfilePhoto,
            ),
            ("status", "about", PrivacySettingCategory::About),
            ("groupadd", "group_add", PrivacySettingCategory::GroupAdd),
            ("messages", "messages", PrivacySettingCategory::Messages),
            (
                "readreceipts",
                "read_receipts",
                PrivacySettingCategory::ReadReceipts,
            ),
        ] {
            assert_eq!(
                PrivacySettingCategory::from_wire(wire),
                expected,
                "{wire} is not the category it names"
            );
            assert_eq!(
                expected.to_wire(),
                Some(wire),
                "{expected:?} did not go back to {wire}"
            );
            let json = format!("\"{dto_name}\"");
            let decoded: PrivacySettingCategory =
                serde_json::from_str(&json).expect("a category name decodes");
            assert_eq!(decoded, expected, "{json} changed meaning on the wire");
            assert_eq!(
                serde_json::to_string(&expected).unwrap(),
                json,
                "{expected:?} changed its DTO name"
            );
        }
        assert_eq!(
            PrivacySettingCategory::from_wire("later_setting"),
            PrivacySettingCategory::Unknown
        );
    }

    #[test]
    fn every_documented_value_maps_to_its_dto_and_back() {
        for (wire, expected) in [
            ("all", PrivacySettingValue::All),
            ("contacts", PrivacySettingValue::Contacts),
            ("none", PrivacySettingValue::None),
            ("match_last_seen", PrivacySettingValue::MatchLastSeen),
            ("contact_blacklist", PrivacySettingValue::ContactBlacklist),
        ] {
            assert_eq!(
                PrivacySettingValue::from_wire(wire),
                expected,
                "{wire} is not the value it names"
            );
            assert_eq!(
                expected.to_wire(),
                Some(wire),
                "{expected:?} did not go back to {wire}"
            );
            let json = format!("\"{wire}\"");
            let decoded: PrivacySettingValue =
                serde_json::from_str(&json).expect("a value decodes");
            assert_eq!(decoded, expected, "{json} changed meaning on the wire");
            assert_eq!(
                serde_json::to_string(&expected).unwrap(),
                json,
                "{expected:?} changed its DTO name"
            );
        }
        assert_eq!(
            PrivacySettingValue::from_wire("later_value"),
            PrivacySettingValue::Unknown
        );
        for settable in [
            PrivacySettingValue::All,
            PrivacySettingValue::Contacts,
            PrivacySettingValue::None,
            PrivacySettingValue::MatchLastSeen,
        ] {
            assert!(settable.is_settable(), "{settable:?} should be settable");
        }
        for read_only in [
            PrivacySettingValue::ContactBlacklist,
            PrivacySettingValue::Unknown,
        ] {
            assert!(
                !read_only.is_settable(),
                "{read_only:?} must not reach the plain setter"
            );
        }
    }

    #[test]
    fn an_unknown_dto_has_no_upstream_wire_name() {
        assert_eq!(PrivacySettingCategory::Unknown.to_wire(), None);
        assert_eq!(PrivacySettingValue::Unknown.to_wire(), None);
    }

    #[test]
    fn accepts_rejects_what_the_plain_setter_cannot_send() {
        let categories = [
            PrivacySettingCategory::LastSeen,
            PrivacySettingCategory::Online,
            PrivacySettingCategory::ProfilePhoto,
            PrivacySettingCategory::About,
            PrivacySettingCategory::GroupAdd,
            PrivacySettingCategory::Messages,
            PrivacySettingCategory::ReadReceipts,
            PrivacySettingCategory::Unknown,
        ];
        for category in categories {
            assert!(
                !category.accepts(PrivacySettingValue::ContactBlacklist),
                "{category:?} must not accept a disallowed-list value"
            );
            assert!(
                !category.accepts(PrivacySettingValue::Unknown),
                "{category:?} must not accept an unknown value"
            );
            if category != PrivacySettingCategory::Online {
                assert!(
                    !category.accepts(PrivacySettingValue::MatchLastSeen),
                    "{category:?} must not borrow the online setting"
                );
            }
        }
        for category in [
            PrivacySettingCategory::LastSeen,
            PrivacySettingCategory::ProfilePhoto,
            PrivacySettingCategory::About,
            PrivacySettingCategory::GroupAdd,
        ] {
            for value in [
                PrivacySettingValue::All,
                PrivacySettingValue::Contacts,
                PrivacySettingValue::None,
            ] {
                assert!(
                    category.accepts(value),
                    "{category:?} should accept {value:?}"
                );
            }
        }
        for value in [PrivacySettingValue::All, PrivacySettingValue::MatchLastSeen] {
            assert!(PrivacySettingCategory::Online.accepts(value));
        }
        for value in [PrivacySettingValue::All, PrivacySettingValue::Contacts] {
            assert!(PrivacySettingCategory::Messages.accepts(value));
        }
        for value in [PrivacySettingValue::All, PrivacySettingValue::None] {
            assert!(PrivacySettingCategory::ReadReceipts.accepts(value));
        }
    }

    #[test]
    fn privacy_settings_round_trip_and_keep_unknown_fields_unknown() {
        let setting = PrivacySetting {
            category: PrivacySettingCategory::GroupAdd,
            value: PrivacySettingValue::Contacts,
        };
        let settings = PrivacySettings {
            settings: vec![
                setting,
                PrivacySetting {
                    category: PrivacySettingCategory::Unknown,
                    value: PrivacySettingValue::Unknown,
                },
            ],
        };
        let line = serde_json::to_string(&settings).unwrap();
        assert_eq!(
            line,
            r#"{"settings":[{"category":"group_add","value":"contacts"},{"category":"unknown","value":"unknown"}]}"#
        );
        assert_eq!(
            serde_json::from_str::<PrivacySettings>(&line).unwrap(),
            settings
        );
        assert_eq!(
            settings.get(PrivacySettingCategory::GroupAdd),
            Some(&PrivacySetting {
                category: PrivacySettingCategory::GroupAdd,
                value: PrivacySettingValue::Contacts,
            })
        );
        assert_eq!(settings.get(PrivacySettingCategory::LastSeen), None);

        #[derive(Deserialize)]
        struct LegacyFrame {
            #[serde(default)]
            category: PrivacySettingCategory,
            #[serde(default)]
            value: PrivacySettingValue,
        }
        let missing: LegacyFrame = serde_json::from_str("{}").unwrap();
        assert_eq!(missing.category, PrivacySettingCategory::Unknown);
        assert_eq!(missing.value, PrivacySettingValue::Unknown);
        let newer: LegacyFrame =
            serde_json::from_str(r#"{"category":"later_setting","value":"later_value"}"#).unwrap();
        assert_eq!(newer.category, PrivacySettingCategory::Unknown);
        assert_eq!(newer.value, PrivacySettingValue::Unknown);
        let half_known: PrivacySetting =
            serde_json::from_str(r#"{"category":"later_setting","value":"contacts"}"#).unwrap();
        assert_eq!(half_known.category, PrivacySettingCategory::Unknown);
        assert_eq!(half_known.value, PrivacySettingValue::Contacts);
    }
}
