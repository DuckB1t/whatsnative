//! The account's DM read-receipts setting, as a read and a mutation.
//!
//! This is one category out of the server's privacy list. It has two
//! documented values, `all` and `none`; anything else is kept as unknown rather
//! than read as "on". The other categories are deliberately absent here: a
//! setter for read receipts says nothing about status receipts, group receipts,
//! story audience, last seen or About visibility.
//!
//! A SET IQ answers with only a `dhash`, so this module does the honest thing
//! after a successful mutation: write the same state the upstream login path
//! keeps, flush it, and read the category back from the server. The front end
//! changes what it draws only when that fresh answer gets published.

use oxidezap_core::PrivacyReadReceipts;
use whatsapp_rust::wacore::iq::privacy::{PrivacyCategory, PrivacySettingsResponse, PrivacyValue};
use whatsapp_rust::wacore::store::commands::DeviceCommand;

use super::WhatsAppClient;
use crate::exec::Task;

impl WhatsAppClient {
    /// Read the server's current DM read-receipts value.
    ///
    /// There is no local copy to query: login keeps only whether receipts were
    /// disabled inside the device snapshot, and that is the session's store.
    pub fn fetch_read_receipts(&self) -> Task<Result<PrivacyReadReceipts, String>> {
        let session = self.session.clone();
        self.exec.spawn(async move {
            let Some(live) = session.lock().await.clone() else {
                return Err("no session yet".to_string());
            };
            let settings = live
                .client
                .fetch_privacy_settings()
                .await
                .map_err(|error| error.to_string())?;
            Ok(read_receipts_from(&settings))
        })
    }

    /// Set account-level DM read receipts and return the value the server then
    /// reports.
    ///
    /// The device snapshot is updated as well as the server, because the
    /// upstream login path treats that field as the account's last known
    /// answer. Both the write and the read-back are awaited here; `true` and
    /// `false` are the only values this category accepts.
    pub fn set_read_receipts(&self, enabled: bool) -> Task<Result<PrivacyReadReceipts, String>> {
        let session = self.session.clone();
        self.exec.spawn(async move {
            let Some(live) = session.lock().await.clone() else {
                return Err("no session yet".to_string());
            };
            live.client
                .set_privacy_setting(PrivacyCategory::ReadReceipts, privacy_value(enabled))
                .await
                .map_err(|error| error.to_string())?;
            // The SET response carries only a dhash. Keep the local snapshot
            // in step with the value that was sent, then make it durable before
            // claiming the account changed.
            live.client
                .persistence_manager()
                .process_command(DeviceCommand::SetReadReceiptsDisabled(!enabled))
                .await;
            live.client
                .persistence_manager()
                .flush()
                .await
                .map_err(|error| format!("could not save the read-receipts change: {error}"))?;

            let fresh = live
                .client
                .fetch_privacy_settings()
                .await
                .map_err(|error| {
                    format!("the change may have been saved, but the server could not be asked again: {error}")
                })?;
            let observed = read_receipts_from(&fresh);
            if observed != expected(enabled) {
                return Err(format!(
                    "WhatsApp did not report the requested read-receipts value (got {})",
                    label(observed)
                ));
            }
            Ok(observed)
        })
    }
}

/// One wire value as the domain DTO, keeping anything unexpected as unknown.
fn read_receipts_from(settings: &PrivacySettingsResponse) -> PrivacyReadReceipts {
    match settings.get_value(&PrivacyCategory::ReadReceipts) {
        Some(PrivacyValue::All) => PrivacyReadReceipts::All,
        Some(PrivacyValue::None) => PrivacyReadReceipts::None,
        _ => PrivacyReadReceipts::Unknown,
    }
}

/// The only two values this category accepts.
fn privacy_value(enabled: bool) -> PrivacyValue {
    if enabled {
        PrivacyValue::All
    } else {
        PrivacyValue::None
    }
}

fn expected(enabled: bool) -> PrivacyReadReceipts {
    if enabled {
        PrivacyReadReceipts::All
    } else {
        PrivacyReadReceipts::None
    }
}

fn label(value: PrivacyReadReceipts) -> &'static str {
    match value {
        PrivacyReadReceipts::All => "all",
        PrivacyReadReceipts::None => "none",
        PrivacyReadReceipts::Unknown => "an unrecognised value",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Synthetic server answers only. The point is that a value from a newer
    /// server is never quietly drawn as "on".
    #[test]
    fn a_missing_or_unknown_category_stays_unknown() {
        let empty = PrivacySettingsResponse::default();
        assert_eq!(read_receipts_from(&empty), PrivacyReadReceipts::Unknown);

        let other = PrivacySettingsResponse {
            settings: vec![whatsapp_rust::wacore::iq::privacy::PrivacySetting {
                category: PrivacyCategory::Last,
                value: PrivacyValue::All,
            }],
        };
        assert_eq!(read_receipts_from(&other), PrivacyReadReceipts::Unknown);
    }

    #[test]
    fn the_two_documented_values_map_in_both_directions() {
        let value = |wire: &str| {
            PrivacyReadReceipts::from_wire(wire)
                == match wire {
                    "all" => PrivacyReadReceipts::All,
                    "none" => PrivacyReadReceipts::None,
                    _ => unreachable!("only documented values are used here"),
                }
        };
        assert!(value("all"));
        assert!(value("none"));

        for (enabled, dto, wire) in [
            (true, PrivacyReadReceipts::All, "all"),
            (false, PrivacyReadReceipts::None, "none"),
        ] {
            assert_eq!(expected(enabled), dto);
            assert_eq!(label(dto), wire);
            let settings = PrivacySettingsResponse {
                settings: vec![whatsapp_rust::wacore::iq::privacy::PrivacySetting {
                    category: PrivacyCategory::ReadReceipts,
                    value: privacy_value(enabled),
                }],
            };
            assert_eq!(
                read_receipts_from(&settings),
                dto,
                "a synthetic read-back must agree with what was sent"
            );
        }
    }
}
