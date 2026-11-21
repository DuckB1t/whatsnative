//! Community and subgroup vocabulary.
//!
//! Requests and their outcomes as plain strings and collections: no library
//! group types, no network calls, no account state. Every optional and
//! collection field defaults so an older or additive frame still decodes, and
//! partial failures stay reportable rather than aborting a whole operation.

use serde::{Deserialize, Serialize};

fn is_false(value: &bool) -> bool {
    !*value
}

/// Whether a community or subgroup is listed for the user.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommunityVisibility {
    Visible,
    Hidden,
}

/// What creating a community was asked to do.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommunityDraft {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub closed: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub allow_non_admin_sub_group_creation: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub create_general_chat: bool,
}

/// A subgroup to create under an existing community.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubgroupDraft {
    pub parent_jid: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub participants: Vec<String>,
    pub visibility: CommunityVisibility,
}

/// One group to attach to a community, with the listing it should get.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LinkSubgroupTarget {
    pub jid: String,
    pub visibility: CommunityVisibility,
}

/// Attach existing groups to a community in one operation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LinkSubgroupsRequest {
    pub community_jid: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub groups: Vec<LinkSubgroupTarget>,
}

/// Ask to join a community's subgroup.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JoinSubgroupRequest {
    pub community_jid: String,
    pub subgroup_jid: String,
}

/// A group as the community list knows it. The subject and description are
/// what the server last said, not a local rename.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommunityGroupRef {
    pub jid: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subject: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// One group an operation could not handle, so the rest can still be reported.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommunitySubgroupFailure {
    pub jid: String,
    pub code: u32,
}

/// Which step a non-fatal warning came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommunityWarningStep {
    SetDescription,
    LinkSubgroup,
    JoinApprovalPending,
}

/// Something that did not complete but did not fail the whole operation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommunityWarning {
    pub step: CommunityWarningStep,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub jid: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// The whole answer to a community operation, including its partial failures.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct CommunityOperationResult {
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub groups: Vec<CommunityGroupRef>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub linked_jids: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub failed_groups: Vec<CommunitySubgroupFailure>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<CommunityWarning>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_operation_result_round_trips_with_its_failures() {
        let result = CommunityOperationResult {
            groups: vec![CommunityGroupRef {
                jid: "155555500001@g.us".into(),
                subject: Some("Synthetic group".into()),
                description: None,
            }],
            linked_jids: vec!["155555500002@g.us".into()],
            failed_groups: vec![CommunitySubgroupFailure {
                jid: "155555500003@g.us".into(),
                code: 403,
            }],
            warnings: vec![CommunityWarning {
                step: CommunityWarningStep::JoinApprovalPending,
                jid: Some("155555500004@g.us".into()),
                detail: None,
            }],
        };
        let encoded = serde_json::to_value(&result).unwrap();
        assert_eq!(
            serde_json::from_value::<CommunityOperationResult>(encoded).unwrap(),
            result
        );
    }

    #[test]
    fn an_old_or_empty_frame_decodes_to_empty_collections() {
        let result: CommunityOperationResult = serde_json::from_str("{}").unwrap();
        assert_eq!(result, CommunityOperationResult::default());
        assert!(result.groups.is_empty());
        assert!(result.linked_jids.is_empty());
        assert!(result.failed_groups.is_empty());
        assert!(result.warnings.is_empty());
    }

    #[test]
    fn drafts_and_warnings_default_their_optional_parts() {
        let draft: CommunityDraft = serde_json::from_str(r#"{"name":"Synthetic"}"#).unwrap();
        assert_eq!(
            draft,
            CommunityDraft {
                name: "Synthetic".into(),
                description: None,
                closed: false,
                allow_non_admin_sub_group_creation: false,
                create_general_chat: false,
            }
        );
        let sub: SubgroupDraft = serde_json::from_str(
            r#"{"parent_jid":"155555500001@g.us","name":"Synthetic","visibility":"hidden"}"#,
        )
        .unwrap();
        assert!(sub.participants.is_empty());
        assert_eq!(sub.visibility, CommunityVisibility::Hidden);

        let warning: CommunityWarning =
            serde_json::from_str(r#"{"step":"set_description"}"#).unwrap();
        assert_eq!(warning.step, CommunityWarningStep::SetDescription);
        assert_eq!(warning.jid, None);
        assert_eq!(warning.detail, None);
    }

    #[test]
    fn visibility_and_warning_steps_use_snake_case() {
        assert_eq!(
            serde_json::to_string(&CommunityVisibility::Hidden).unwrap(),
            "\"hidden\""
        );
        assert_eq!(
            serde_json::to_string(&CommunityWarningStep::LinkSubgroup).unwrap(),
            "\"link_subgroup\""
        );
        assert_eq!(
            serde_json::to_string(&CommunityWarningStep::JoinApprovalPending).unwrap(),
            "\"join_approval_pending\""
        );
    }
}
