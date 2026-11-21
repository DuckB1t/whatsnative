//! Resolve a Meta username against the live account, as domain data.
//!
//! The upstream lookup is experimental: the request matches WhatsApp Web but
//! no recorded server answer backs it. Its three server outcomes, plus local
//! validation of the handle, are kept distinct on the way out — a handle
//! needing a key is not a missing handle — and a library error never becomes
//! a line of GUI text. Only the address book's
//! name and the verified name carried by the response are surfaced.

use oxidezap_core::{ResolvedUsername, UsernameLookupResult, normalize_username};
use whatsapp_rust::features::{
    ContactError, UsernameLookup, UsernameLookupError, UsernameLookupUser,
};

use super::{Session, WhatsAppClient};
use crate::exec::Task;

impl WhatsAppClient {
    /// Look up a handle, optionally with the account's numeric username key.
    ///
    /// A `KeyRequired` answer asks for a second call with that key; the daemon
    /// passes the typed answer through unchanged so the front end can say
    /// which of the three happened rather than guessing from a missing name.
    pub fn find_by_username(
        &self,
        username: String,
        username_key: Option<String>,
    ) -> Task<Result<UsernameLookupResult, String>> {
        let session = self.session.clone();
        self.exec.spawn(async move {
            let Some(username) = normalize_username(&username) else {
                return Ok(UsernameLookupResult::InvalidUsername);
            };
            let Some(live) = session.lock().await.clone() else {
                return Err("no session yet".to_string());
            };
            let lookup = match live
                .client
                .contacts()
                .find_by_username(&username, username_key.as_deref())
                .await
            {
                Ok(lookup) => lookup,
                // A rejected handle is a typed answer, not a server failure:
                // the same limit is checked before the request is sent, but a
                // front end must still be told plainly if it slips through.
                Err(ContactError::Username(UsernameLookupError::InvalidLength { .. })) => {
                    return Ok(UsernameLookupResult::InvalidUsername);
                }
                Err(error) => return Err(error.to_string()),
            };
            domain_from(&live, lookup).await
        })
    }
}

/// A server answer becomes one of the three shapes a front end can draw.
///
/// The local name is looked up here, where the address book lives. It is
/// never the handle: a username is not a name somebody gave this account.
async fn domain_from(
    live: &Session,
    lookup: UsernameLookup,
) -> Result<UsernameLookupResult, String> {
    match lookup {
        UsernameLookup::Found(user) => {
            let name = live.names.known(&live.client, &user.jid, None).await;
            let mut resolved = resolved_from(user);
            resolved.name = name;
            Ok(UsernameLookupResult::Found(resolved))
        }
        other => other_answer(other),
    }
}

/// The answers that need no address book, kept apart so they can be tested.
fn other_answer(lookup: UsernameLookup) -> Result<UsernameLookupResult, String> {
    match lookup {
        UsernameLookup::KeyRequired { username } => Ok(UsernameLookupResult::KeyRequired {
            username: username.map(|name| name.to_string()),
        }),
        UsernameLookup::NotFound => Ok(UsernameLookupResult::NotFound),
        _ => Err("the session could not understand this username answer".to_string()),
    }
}

/// The protocol's own fields, with no name added and no verified name invented.
fn resolved_from(user: UsernameLookupUser) -> ResolvedUsername {
    ResolvedUsername {
        jid: user.jid.to_non_ad_string(),
        pn_jid: user.pn_jid.map(|jid| jid.to_non_ad_string()),
        username: user.username.map(|name| name.to_string()),
        name: None,
        verified_name: user.verified_name.and_then(|verified| verified.name),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use whatsapp_rust::wacore::iq::{spec::IqSpec, usync::UsernameLookupSpec};
    use whatsapp_rust::wacore_binary::{Node, builder::NodeBuilder};

    /// The upstream type is non-exhaustive, so fixtures are parsed from a
    /// synthetic IQ rather than built by hand. No address or name here belongs
    /// to a real account.
    fn parsed(users: Vec<Node>) -> UsernameLookup {
        let spec = UsernameLookupSpec::new("example.handle", None, "sid-1").unwrap();
        let response = NodeBuilder::new("iq")
            .attr("type", "result")
            .children([NodeBuilder::new("usync")
                .children([NodeBuilder::new("list").children(users).build()])
                .build()])
            .build();
        spec.parse_response(&response.as_node_ref()).unwrap()
    }

    #[test]
    fn a_found_answer_keeps_the_server_address_and_verified_name() {
        let lookup = parsed(vec![
            NodeBuilder::new("user")
                .attr("jid", "100000001@lid")
                .children([
                    NodeBuilder::new("contact")
                        .attr("type", "in")
                        .attr("username", "example.handle")
                        .build(),
                    NodeBuilder::new("business")
                        .attr("pn_jid", "15555550100@s.whatsapp.net")
                        .children([NodeBuilder::new("verified_name")
                            .attr("name", "Example Store")
                            .build()])
                        .build(),
                ])
                .build(),
        ]);
        let UsernameLookup::Found(user) = lookup else {
            panic!("expected a resolved account");
        };
        let resolved = resolved_from(user);
        assert_eq!(resolved.jid, "100000001@lid");
        assert_eq!(
            resolved.pn_jid.as_deref(),
            Some("15555550100@s.whatsapp.net")
        );
        assert_eq!(resolved.username.as_deref(), Some("example.handle"));
        assert_eq!(resolved.verified_name.as_deref(), Some("Example Store"));
        assert_eq!(resolved.name, None, "a name belongs to the address book");
    }

    #[test]
    fn a_certificate_without_a_name_does_not_invent_one() {
        let lookup = parsed(vec![
            NodeBuilder::new("user")
                .attr("jid", "100000001@lid")
                .children([
                    NodeBuilder::new("contact").attr("type", "in").build(),
                    NodeBuilder::new("business")
                        .children([NodeBuilder::new("verified_name")
                            .attr("serial", "serial-1")
                            .build()])
                        .build(),
                ])
                .build(),
        ]);
        let UsernameLookup::Found(user) = lookup else {
            panic!("expected a resolved account");
        };
        assert_eq!(resolved_from(user).verified_name, None);
    }

    #[test]
    fn the_other_two_answers_stay_distinct() {
        let missing = parsed(vec![
            NodeBuilder::new("user")
                .attr("jid", "100000001@lid")
                .children([NodeBuilder::new("contact").attr("type", "out").build()])
                .build(),
        ]);
        assert_eq!(
            other_answer(missing).unwrap(),
            UsernameLookupResult::NotFound
        );

        let key_required = parsed(vec![
            NodeBuilder::new("user")
                .children([NodeBuilder::new("contact")
                    .attr("type", "in")
                    .attr("username", "example.handle")
                    .build()])
                .build(),
        ]);
        assert_eq!(
            other_answer(key_required).unwrap(),
            UsernameLookupResult::KeyRequired {
                username: Some("example.handle".to_string()),
            }
        );
    }
}
