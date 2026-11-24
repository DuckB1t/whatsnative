//! The account's server-owned privacy settings, read and changed on request.
//!
//! DM read receipts keeps its dedicated three-state DTO and its own protocol 60
//! path. It has two documented values, `all` and `none`; anything else is kept
//! as unknown rather than read as "on". A setter for read receipts says nothing
//! about status receipts, group receipts or story audience.
//!
//! The generic categories use the DTOs in `oxidezap_core`. Upstream rows are
//! mapped as they arrive, so a name or value this build does not know stays
//! unknown, and a category the server omits is not invented.
//!
//! A SET IQ answers with only a `dhash`. After a read-receipts mutation this
//! module writes the state the upstream login path keeps, flushes it, and reads
//! the category back from the server. The generic setter writes no device
//! state: it validates the pair, sends the IQ, then asks the server again and
//! checks that the requested category reports the requested value. The front
//! end changes what it draws only when a fresh answer gets published.

use oxidezap_core::{
    PrivacyReadReceipts, PrivacySetting, PrivacySettingCategory, PrivacySettingValue,
    PrivacySettings,
};
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

    /// Read the server's generic privacy categories as core DTOs.
    ///
    /// There is no local copy of these settings. Every row comes from the
    /// server, and unknown names and values stay unknown rather than becoming
    /// a guess at a known setting.
    pub fn fetch_privacy_categories(&self) -> Task<Result<PrivacySettings, String>> {
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
            Ok(privacy_settings_from(&settings))
        })
    }

    /// Change one of the six generic privacy categories and return the fresh
    /// server answer.
    ///
    /// Read receipts is not in this setter: protocol 60 owns it and keeps a
    /// device snapshot. The pair is validated before any network work, and the
    /// SET response is only a dhash, so the server is asked again and the
    /// requested value must be reported before this returns success.
    pub fn set_privacy_category(
        &self,
        category: PrivacySettingCategory,
        value: PrivacySettingValue,
    ) -> Task<Result<PrivacySettings, String>> {
        let (upstream_category, upstream_value) = match validate_generic_pair(category, value) {
            Ok(pair) => pair,
            Err(error) => return self.exec.spawn(async move { Err(error) }),
        };
        let session = self.session.clone();
        self.exec.spawn(async move {
            let Some(live) = session.lock().await.clone() else {
                return Err("no session yet".to_string());
            };
            live.client
                .set_privacy_setting(upstream_category, upstream_value)
                .await
                .map_err(|error| error.to_string())?;
            // The SET response carries only a dhash. Ask the server again
            // rather than trusting a local guess about what changed.
            let fresh = live
                .client
                .fetch_privacy_settings()
                .await
                .map_err(|error| {
                    format!("the change may have been sent, but the server could not be asked again: {error}")
                })?;
            let settings = privacy_settings_from(&fresh);
            let observed = settings.get(category).map(|setting| setting.value);
            if observed != Some(value) {
                return Err(format!(
                    "WhatsApp did not report the requested value for {category:?}: asked for {value:?}, observed {observed:?}"
                ));
            }
            Ok(settings)
        })
    }
}

/// One upstream answer as the core DTO, keeping unknown rows unknown.
fn privacy_settings_from(response: &PrivacySettingsResponse) -> PrivacySettings {
    PrivacySettings {
        settings: response
            .settings
            .iter()
            .map(|setting| PrivacySetting {
                category: PrivacySettingCategory::from_wire(setting.category.as_str()),
                value: PrivacySettingValue::from_wire(setting.value.as_str()),
            })
            .collect(),
    }
}

/// Translate one DTO pair into the upstream spelling, or refuse it.
///
/// `ContactBlacklist` is read but never sent here: changing it needs a
/// disallowed-list update. `Unknown` has no upstream name. Read receipts is
/// refused as a category because its dedicated path owns the device snapshot.
fn upstream_pair(
    category: PrivacySettingCategory,
    value: PrivacySettingValue,
) -> Option<(PrivacyCategory, PrivacyValue)> {
    let category = match category {
        PrivacySettingCategory::LastSeen => PrivacyCategory::Last,
        PrivacySettingCategory::Online => PrivacyCategory::Online,
        PrivacySettingCategory::ProfilePhoto => PrivacyCategory::Profile,
        PrivacySettingCategory::About => PrivacyCategory::Status,
        PrivacySettingCategory::GroupAdd => PrivacyCategory::GroupAdd,
        PrivacySettingCategory::Messages => PrivacyCategory::Messages,
        PrivacySettingCategory::ReadReceipts | PrivacySettingCategory::Unknown => return None,
    };
    let value = match value {
        PrivacySettingValue::All => PrivacyValue::All,
        PrivacySettingValue::Contacts => PrivacyValue::Contacts,
        PrivacySettingValue::None => PrivacyValue::None,
        PrivacySettingValue::MatchLastSeen => PrivacyValue::MatchLastSeen,
        PrivacySettingValue::ContactBlacklist | PrivacySettingValue::Unknown => return None,
    };
    Some((category, value))
}

/// Every gate a generic privacy change has to pass before it reaches the wire.
fn validate_generic_pair(
    category: PrivacySettingCategory,
    value: PrivacySettingValue,
) -> Result<(PrivacyCategory, PrivacyValue), String> {
    if !matches!(
        category,
        PrivacySettingCategory::LastSeen
            | PrivacySettingCategory::Online
            | PrivacySettingCategory::ProfilePhoto
            | PrivacySettingCategory::About
            | PrivacySettingCategory::GroupAdd
            | PrivacySettingCategory::Messages
    ) {
        return Err(format!(
            "{category:?} has no generic setter; read receipts use their own path"
        ));
    }
    if !category.accepts(value) || !value.is_settable() {
        return Err(format!("{category:?} does not accept {value:?}"));
    }
    if category.to_wire().is_none() || value.to_wire().is_none() {
        return Err("a privacy change needs an upstream name for its category and value".into());
    }
    let (upstream_category, upstream_value) = upstream_pair(category, value)
        .ok_or_else(|| format!("{category:?} cannot be sent as {value:?}"))?;
    if !upstream_category.is_valid_value(&upstream_value) {
        return Err(format!(
            "WhatsApp does not allow {category:?} to be set to {value:?}"
        ));
    }
    Ok((upstream_category, upstream_value))
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

    /// Synthetic upstream rows only. `calladd` and a future row have no core
    /// category here, so neither may be relabelled as one that is known.
    #[test]
    fn synthetic_upstream_rows_keep_unknown_categories_and_values_unknown() {
        let response = PrivacySettingsResponse {
            settings: vec![
                whatsapp_rust::wacore::iq::privacy::PrivacySetting {
                    category: PrivacyCategory::Last,
                    value: PrivacyValue::Contacts,
                },
                whatsapp_rust::wacore::iq::privacy::PrivacySetting {
                    category: PrivacyCategory::Online,
                    value: PrivacyValue::MatchLastSeen,
                },
                whatsapp_rust::wacore::iq::privacy::PrivacySetting {
                    category: PrivacyCategory::Messages,
                    value: PrivacyValue::All,
                },
                whatsapp_rust::wacore::iq::privacy::PrivacySetting {
                    category: PrivacyCategory::CallAdd,
                    value: PrivacyValue::Known,
                },
                whatsapp_rust::wacore::iq::privacy::PrivacySetting {
                    category: PrivacyCategory::from("synthetic_future_category"),
                    value: PrivacyValue::from("synthetic_future_value"),
                },
            ],
        };

        let mapped = privacy_settings_from(&response);
        assert_eq!(mapped.settings.len(), 5);
        assert_eq!(
            mapped.get(PrivacySettingCategory::LastSeen),
            Some(&PrivacySetting {
                category: PrivacySettingCategory::LastSeen,
                value: PrivacySettingValue::Contacts,
            })
        );
        assert_eq!(
            mapped.get(PrivacySettingCategory::Online),
            Some(&PrivacySetting {
                category: PrivacySettingCategory::Online,
                value: PrivacySettingValue::MatchLastSeen,
            })
        );
        assert_eq!(
            mapped.get(PrivacySettingCategory::Messages),
            Some(&PrivacySetting {
                category: PrivacySettingCategory::Messages,
                value: PrivacySettingValue::All,
            })
        );
        let unknown_rows: Vec<_> = mapped
            .settings
            .iter()
            .filter(|row| row.category == PrivacySettingCategory::Unknown)
            .collect();
        assert_eq!(
            unknown_rows.len(),
            2,
            "calladd and the future row stay unknown"
        );
        for row in unknown_rows {
            assert_eq!(row.value, PrivacySettingValue::Unknown);
        }
    }

    /// Every allowed category and every settable value has an explicit upstream
    /// spelling. Nothing here falls through to `Other` or a guessed name.
    #[test]
    fn the_generic_setter_maps_six_categories_and_four_values() {
        use super::{PrivacySettingCategory as Category, PrivacySettingValue as Value};

        for (category, value, upstream_category, upstream_value) in [
            (
                Category::LastSeen,
                Value::All,
                PrivacyCategory::Last,
                PrivacyValue::All,
            ),
            (
                Category::LastSeen,
                Value::Contacts,
                PrivacyCategory::Last,
                PrivacyValue::Contacts,
            ),
            (
                Category::LastSeen,
                Value::None,
                PrivacyCategory::Last,
                PrivacyValue::None,
            ),
            (
                Category::Online,
                Value::All,
                PrivacyCategory::Online,
                PrivacyValue::All,
            ),
            (
                Category::Online,
                Value::MatchLastSeen,
                PrivacyCategory::Online,
                PrivacyValue::MatchLastSeen,
            ),
            (
                Category::ProfilePhoto,
                Value::Contacts,
                PrivacyCategory::Profile,
                PrivacyValue::Contacts,
            ),
            (
                Category::About,
                Value::None,
                PrivacyCategory::Status,
                PrivacyValue::None,
            ),
            (
                Category::GroupAdd,
                Value::All,
                PrivacyCategory::GroupAdd,
                PrivacyValue::All,
            ),
            (
                Category::Messages,
                Value::Contacts,
                PrivacyCategory::Messages,
                PrivacyValue::Contacts,
            ),
        ] {
            assert!(category.to_wire().is_some(), "{category:?} has a wire name");
            assert!(value.to_wire().is_some(), "{value:?} has a wire name");
            assert_eq!(
                validate_generic_pair(category, value),
                Ok((upstream_category, upstream_value)),
                "{category:?} with {value:?} has the explicit upstream pair"
            );
        }
    }

    /// The dedicated read-receipts path owns its category, a disallowed list is
    /// not a plain setter value, and upstream refuses pairs the core gate also
    /// refuses. None of these may reach an IQ.
    #[test]
    fn the_generic_setter_refuses_pairs_it_cannot_send() {
        use super::{PrivacySettingCategory as Category, PrivacySettingValue as Value};

        for (category, value) in [
            (Category::ReadReceipts, Value::All),
            (Category::Unknown, Value::All),
            (Category::LastSeen, Value::ContactBlacklist),
            (Category::LastSeen, Value::Unknown),
            (Category::LastSeen, Value::MatchLastSeen),
            (Category::Online, Value::Contacts),
            (Category::Messages, Value::None),
        ] {
            assert!(
                validate_generic_pair(category, value).is_err(),
                "{category:?} with {value:?} must be refused before a send"
            );
        }
        assert!(!PrivacyCategory::Last.is_valid_value(&PrivacyValue::MatchLastSeen));
        assert!(validate_generic_pair(Category::LastSeen, Value::MatchLastSeen).is_err());
        assert!(!PrivacyCategory::Online.is_valid_value(&PrivacyValue::Contacts));
        assert!(validate_generic_pair(Category::Online, Value::Contacts).is_err());
    }
}
