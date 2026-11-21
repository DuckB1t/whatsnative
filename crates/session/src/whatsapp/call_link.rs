//! Call-link metadata, read without joining the link.
//!
//! A preview is a lookup, not a placement. The token is the invite itself, so
//! nothing here turns it into an address or a call identity: no devices are
//! opened, nothing is written to the call registry, no call row is made, and
//! no log line ever carries the token. What comes back is the link's own
//! metadata — who made it, which media it offers, and whether a waiting room
//! is in front of it.

use super::WhatsAppClient;
use crate::exec::Task;
use oxidezap_core::{CallLinkMedia, CallLinkPreview};
use whatsapp_rust::voip::CallLinkMedia as UpstreamCallLinkMedia;
use whatsapp_rust::wacore_binary::jid::Jid;

/// The front end's media choice, in the vocabulary the call service speaks.
///
/// Kept here, next to the reverse mapping, so a placement and a preview cannot
/// disagree about which way `Audio` and `Video` go.
pub(super) fn upstream_media(media: CallLinkMedia) -> UpstreamCallLinkMedia {
    match media {
        CallLinkMedia::Audio => UpstreamCallLinkMedia::Audio,
        CallLinkMedia::Video => UpstreamCallLinkMedia::Video,
    }
}

fn domain_media(media: UpstreamCallLinkMedia) -> CallLinkMedia {
    match media {
        UpstreamCallLinkMedia::Audio => CallLinkMedia::Audio,
        UpstreamCallLinkMedia::Video => CallLinkMedia::Video,
    }
}

/// The metadata a preview answers with, with JIDs rendered as chat addresses.
///
/// Pure on purpose: the async method below does the I/O, and this is the whole
/// translation. A preview carries no call id, no device and no registry entry
/// — the token it echoes is the link's identity as the server normalized it,
/// not a recipient a call would be placed to.
fn domain_preview(
    token: String,
    media: UpstreamCallLinkMedia,
    creator: &Jid,
    creator_pn: Option<&Jid>,
    waiting_room_enabled: bool,
    is_admin: bool,
) -> CallLinkPreview {
    CallLinkPreview {
        token,
        media: domain_media(media),
        creator_jid: creator.to_non_ad_string(),
        creator_pn: creator_pn.map(Jid::to_non_ad_string),
        waiting_room_enabled,
        is_admin,
    }
}

impl WhatsAppClient {
    /// Read a call link's metadata without joining it or placing a call.
    ///
    /// The token is passed to the uplink as an opaque invite; it is never
    /// parsed as a JID and never logged. This is metadata only, so the session
    /// needs to be open but the call stack needs nothing from this side: no
    /// microphone, camera, registry entry or call row is involved.
    pub fn preview_call_link(
        &self,
        token_or_url: String,
        media: CallLinkMedia,
    ) -> Task<Result<CallLinkPreview, String>> {
        let session = self.session.clone();
        self.exec.spawn(async move {
            let Some(live) = session.lock().await.clone() else {
                return Err("no session yet".to_string());
            };
            let preview = live
                .client
                .voip()
                .preview_call_link(&token_or_url, upstream_media(media))
                .await
                .map_err(|error| error.to_string())?;
            Ok(domain_preview(
                preview.token,
                preview.media,
                &preview.creator,
                preview.creator_pn.as_ref(),
                preview.waiting_room_enabled,
                preview.is_admin,
            ))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxidezap_core::CALL_LINK_RECIPIENT_JID;

    #[cfg_attr(not(target_family = "wasm"), test)]
    #[cfg_attr(target_family = "wasm", wasm_bindgen_test::wasm_bindgen_test)]
    fn preview_call_link_maps_metadata_without_a_call_identity() {
        // A creator seen from another of its devices, and its phone-number
        // alias: both must be rendered as chat addresses, with the device
        // dropped, exactly as every other caller-facing JID is.
        let creator = Jid::pn("15551234567").with_device(7);
        let creator_pn = Jid::lid("200").with_device(3);
        let preview = domain_preview(
            "SYNTHETIC-TOKEN".to_string(),
            UpstreamCallLinkMedia::Video,
            &creator,
            Some(&creator_pn),
            true,
            false,
        );
        assert_eq!(preview.token, "SYNTHETIC-TOKEN");
        assert_eq!(preview.media, CallLinkMedia::Video);
        assert_eq!(preview.creator_jid, "15551234567@s.whatsapp.net");
        assert_eq!(preview.creator_pn.as_deref(), Some("200@lid"));
        assert!(preview.waiting_room_enabled);
        assert!(!preview.is_admin);
        // No JID in the metadata is a device address: nothing here could be
        // mistaken for a call's peer or a registry key.
        assert_eq!(preview.creator_jid.parse::<Jid>().unwrap().device, 0);
        assert_eq!(
            preview
                .creator_pn
                .as_deref()
                .and_then(|value| value.parse::<Jid>().ok())
                .unwrap()
                .device,
            0
        );

        // The mapping is an explicit pair in both directions, not a cast.
        assert_eq!(
            upstream_media(CallLinkMedia::Audio),
            UpstreamCallLinkMedia::Audio
        );
        assert_eq!(
            upstream_media(CallLinkMedia::Video),
            UpstreamCallLinkMedia::Video
        );
        assert_eq!(
            domain_media(UpstreamCallLinkMedia::Audio),
            CallLinkMedia::Audio
        );
        assert_eq!(
            domain_media(UpstreamCallLinkMedia::Video),
            CallLinkMedia::Video
        );
    }

    #[cfg_attr(not(target_family = "wasm"), test)]
    #[cfg_attr(target_family = "wasm", wasm_bindgen_test::wasm_bindgen_test)]
    fn link_tokens_are_never_jids() {
        for token in [
            "SYNTHETIC-LINK-TOKEN",
            "https://call.whatsapp.com/video/SYNTHETIC-LINK-TOKEN",
            "SYNTHETIC.LINK/TOKEN?x=1",
        ] {
            assert!(token.parse::<Jid>().is_err(), "{token} parsed as a JID");
            // A preview echoes the link's identity; it does not reinterpret it
            // as the recipient of a placement.
            let preview = domain_preview(
                token.to_string(),
                UpstreamCallLinkMedia::Audio,
                &Jid::lid("200"),
                None,
                false,
                false,
            );
            assert_eq!(preview.token, token);
            assert_eq!(preview.media, CallLinkMedia::Audio);
        }
        // A link placement is labelled with a constant recipient, never with
        // any part of the invite; that label is not an address either.
        assert_eq!(CALL_LINK_RECIPIENT_JID, "call-link");
        assert!(CALL_LINK_RECIPIENT_JID.parse::<Jid>().is_err());
        assert!(!CALL_LINK_RECIPIENT_JID.contains("SYNTHETIC-LINK-TOKEN"));
    }
}
