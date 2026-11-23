//! One address-book row as it crosses the legacy GUI protocol.
//!
//! This is a projection, not the store's contact record: it carries only what
//! a front end can name and show a person by. The full record, with its labels
//! and business metadata, stays in the store. A row is deliberately made of
//! owned strings with serde defaults so an older daemon's answer still decodes
//! and an older window ignores fields it does not know.

use serde::{Deserialize, Serialize};

/// An address-book hit from a local contact search.
///
/// A search is a local read over the store, so this carries no server total and
/// claims no directory lookup: every row is a person this account already has
/// on disk. The front end uses the address to open or target a conversation; it
/// never invents one.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContactHit {
    /// The contact's address, as the store names it.
    pub jid: String,
    /// The saved name, when there is one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// The name the contact chose for themselves, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub push_name: Option<String>,
    /// The phone number, when the address holds one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phone: Option<String>,
    /// Whether this is a business contact.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub is_business: bool,
    /// The account's own short name for the contact.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alias: Option<String>,
    /// Labels attached to the contact.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
}
