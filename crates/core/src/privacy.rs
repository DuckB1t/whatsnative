//! The one privacy setting this front end can both read and change: account-level
//! direct-message read receipts.
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
}
