//! The account's block list, as a read the daemon can answer.
//!
//! The list lives on the server, not in the chat store: there is no snapshot
//! to fold it into and no event that delivers it. Each call is therefore its
//! own round trip, and the names are looked up here so every surface names a
//! blocked contact the same way a message from them would be named.
//!
//! Nothing in this module guesses a target. [`normalize_block_target`] decides
//! whether what somebody typed is an address the server can block, and
//! block/unblock pass that exact non-device address to the upstream feature.

use oxidezap_core::{BlockedContact, normalize_block_target};
use whatsapp_rust::features::BlocklistEntry;
use whatsapp_rust::wacore_binary::jid::Jid;

use super::WhatsAppClient;
use crate::exec::Task;

impl WhatsAppClient {
    /// Read the block list, naming whoever this account's address book knows.
    pub fn blocklist(&self) -> Task<Result<Vec<BlockedContact>, String>> {
        let session = self.session.clone();
        self.exec.spawn(async move {
            let Some(live) = session.lock().await.clone() else {
                return Err("no session yet".to_string());
            };
            let entries = live
                .client
                .blocking()
                .get_blocklist()
                .await
                .map_err(|error| error.to_string())?;
            let mut contacts = Vec::with_capacity(entries.len());
            for entry in entries {
                // The JID is kept even when a name resolves: Unblock has to
                // send the server's address, including its namespace.
                let name = live.names.known(&live.client, &entry.jid, None).await;
                contacts.push(contact_from(entry, name));
            }
            Ok(contacts)
        })
    }

    /// Put one address on the account's block list.
    ///
    /// The daemon answers only after the server has taken the IQ. A failure is
    /// returned rather than swallowed: a button that looks like it worked
    /// while the list did not change is worse than a visible refusal.
    pub fn block(&self, target: String) -> Task<Result<(), String>> {
        let target = normalize_block_target(&target);
        let session = self.session.clone();
        self.exec.spawn(async move {
            let jid: Jid = target?
                .parse()
                .map_err(|_| "invalid block target".to_string())?;
            let Some(live) = session.lock().await.clone() else {
                return Err("no session yet".to_string());
            };
            live.client
                .blocking()
                .block(&jid)
                .await
                .map_err(|error| error.to_string())
        })
    }

    /// Take one address off the account's block list.
    ///
    /// A LID is accepted without a LID↔PN mapping, matching the upstream
    /// unblock stanza; a PN is resolved through the mapping because the server
    /// needs a LID. Both enter here through the same validation as `block`.
    pub fn unblock(&self, target: String) -> Task<Result<(), String>> {
        let target = normalize_block_target(&target);
        let session = self.session.clone();
        self.exec.spawn(async move {
            let jid: Jid = target?
                .parse()
                .map_err(|_| "invalid block target".to_string())?;
            let Some(live) = session.lock().await.clone() else {
                return Err("no session yet".to_string());
            };
            live.client
                .blocking()
                .unblock(&jid)
                .await
                .map_err(|error| error.to_string())
        })
    }
}

/// A server entry plus the local name resolved for it, as the wire's shape.
fn contact_from(entry: BlocklistEntry, name: Option<String>) -> BlockedContact {
    BlockedContact {
        jid: entry.jid.to_non_ad_string(),
        name,
        blocked_at: entry.timestamp,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_server_entry_keeps_its_address_namespace_and_time() {
        let entry = BlocklistEntry {
            jid: Jid::lid("100000000000001"),
            timestamp: Some(1_700_000_000),
        };
        let contact = contact_from(entry, Some("Ana".to_string()));
        assert_eq!(contact.jid, "100000000000001@lid");
        assert_eq!(contact.label(), "Ana");
        assert_eq!(contact.blocked_at, Some(1_700_000_000));
    }

    #[test]
    fn an_unnamed_entry_has_no_name_and_keeps_its_timestamp_absent() {
        let entry = BlocklistEntry {
            jid: Jid::pn("15555550100"),
            timestamp: None,
        };
        let contact = contact_from(entry, None);
        assert_eq!(contact.jid, "15555550100@s.whatsapp.net");
        assert_eq!(contact.name, None);
        assert_eq!(contact.blocked_at, None);
    }
}
