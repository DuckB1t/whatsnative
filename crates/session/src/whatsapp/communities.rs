//! Native community operations, returned as domain data.
//!
//! The library's community API can succeed in part: the group may be created
//! while a description write fails, a join may be waiting on approval, and a
//! batch link may attach some groups and refuse others. None of those is a
//! hard failure — the caller must be told what exists so it never creates a
//! second community or re-sends a join it already asked for. Everything that
//! is not one of those cases becomes a hard error string.
//!
//! Only synthetic fixtures appear in the tests here; no test touches an
//! account or the network.

use log::warn;
use oxidezap_core::{
    CommunityDraft, CommunityGroupRef, CommunityOperationResult, CommunitySubgroupFailure,
    CommunityVisibility, CommunityWarning, CommunityWarningStep, JoinSubgroupRequest,
    LinkSubgroupsRequest, SubgroupDraft,
};
use whatsapp_rust::wacore_binary::jid::{Jid, JidExt as _};
use whatsapp_rust::{
    CommunityConfigurationStep, CommunityError, CreateCommunityOptions, CreateSubgroupOptions,
    GroupDescription, GroupMetadata, LinkSubgroupOptions, SubgroupFailure, SubgroupVisibility,
};

use super::WhatsAppClient;
use crate::exec::Task;

impl WhatsAppClient {
    /// Create a community. A failed description write still reports the
    /// created group, so a retry cannot duplicate it.
    pub fn create_community(
        &self,
        draft: CommunityDraft,
    ) -> Task<Result<CommunityOperationResult, String>> {
        let session = self.session.clone();
        self.exec.spawn(async move {
            let name = draft.name.trim().to_string();
            if name.is_empty() {
                return Err("a community needs a name".to_string());
            }
            let mut options = CreateCommunityOptions::new(name)
                .map_err(|error| format!("invalid community name: {error}"))?;
            options.closed = draft.closed;
            options.allow_non_admin_sub_group_creation = draft.allow_non_admin_sub_group_creation;
            options.create_general_chat = draft.create_general_chat;
            if let Some(description) = draft.description.as_deref() {
                let description = GroupDescription::new(description)
                    .map_err(|error| format!("invalid community description: {error}"))?;
                options.description = Some(description);
            }
            let Some(live) = session.lock().await.clone() else {
                return Err("no session yet".to_string());
            };
            let answer = live
                .client
                .community()
                .create(options)
                .await
                .map(|created| group_result(&created.metadata));
            settle(answer)
        })
    }

    /// Create a subgroup already linked to a parent, with its add-time
    /// visibility.
    pub fn create_community_subgroup(
        &self,
        draft: SubgroupDraft,
    ) -> Task<Result<CommunityOperationResult, String>> {
        let session = self.session.clone();
        self.exec.spawn(async move {
            let name = draft.name.trim().to_string();
            if name.is_empty() {
                return Err("a subgroup needs a name".to_string());
            }
            let parent = group_jid(&draft.parent_jid, "the parent community")?;
            let mut participants = Vec::with_capacity(draft.participants.len());
            for jid in &draft.participants {
                participants.push(person_jid(jid, "a subgroup participant")?);
            }
            let options = CreateSubgroupOptions::new(name, &participants, parent)
                .with_visibility(visibility(draft.visibility));
            let Some(live) = session.lock().await.clone() else {
                return Err("no session yet".to_string());
            };
            let answer = live
                .client
                .community()
                .create_subgroup_with_options(options)
                .await
                .map(|created| group_result(&created.metadata));
            settle(answer)
        })
    }

    /// Attach groups to a community. One refused group does not hide the ones
    /// that were linked.
    pub fn link_community_subgroups(
        &self,
        request: LinkSubgroupsRequest,
    ) -> Task<Result<CommunityOperationResult, String>> {
        let session = self.session.clone();
        self.exec.spawn(async move {
            let community = group_jid(&request.community_jid, "the community")?;
            let mut options = Vec::with_capacity(request.groups.len());
            for group in &request.groups {
                let jid = group_jid(&group.jid, "a group to link")?;
                options.push(LinkSubgroupOptions::new(jid, visibility(group.visibility)));
            }
            let Some(live) = session.lock().await.clone() else {
                return Err("no session yet".to_string());
            };
            let answer = live
                .client
                .community()
                .link_subgroups_with_options(community, &options)
                .await
                .map(|linked| CommunityOperationResult {
                    linked_jids: linked
                        .linked_jids
                        .into_iter()
                        .map(|jid| jid.to_string())
                        .collect(),
                    failed_groups: linked
                        .failed_groups
                        .into_iter()
                        .map(|failure: SubgroupFailure| CommunitySubgroupFailure {
                            jid: failure.jid.to_string(),
                            code: failure.code,
                        })
                        .collect(),
                    ..Default::default()
                });
            settle(answer)
        })
    }

    /// Join one subgroup. An approval-pending answer is an Ok with a warning,
    /// not a failure: the request reached the community and is still live.
    pub fn join_community_subgroup(
        &self,
        request: JoinSubgroupRequest,
    ) -> Task<Result<CommunityOperationResult, String>> {
        let session = self.session.clone();
        self.exec.spawn(async move {
            let community = group_jid(&request.community_jid, "the community")?;
            let subgroup = group_jid(&request.subgroup_jid, "the subgroup")?;
            let Some(live) = session.lock().await.clone() else {
                return Err("no session yet".to_string());
            };
            let answer = live
                .client
                .community()
                .join_subgroup(community, subgroup)
                .await
                .map(|joined| group_result(&joined));
            settle(answer)
        })
    }
}

fn group_jid(value: &str, what: &str) -> Result<Jid, String> {
    let jid: Jid = value
        .parse()
        .map_err(|error| format!("{what} is not a chat address: {error}"))?;
    if !jid.is_group() {
        return Err(format!("{what} is not a group address"));
    }
    Ok(jid)
}

fn person_jid(value: &str, what: &str) -> Result<Jid, String> {
    value
        .parse()
        .map_err(|error| format!("{what} is not a chat address: {error}"))
}

fn visibility(value: CommunityVisibility) -> SubgroupVisibility {
    match value {
        CommunityVisibility::Visible => SubgroupVisibility::Visible,
        CommunityVisibility::Hidden => SubgroupVisibility::Hidden,
    }
}

fn group_result(metadata: &GroupMetadata) -> CommunityOperationResult {
    CommunityOperationResult {
        groups: vec![CommunityGroupRef {
            jid: metadata.id.to_string(),
            subject: metadata.subject.clone(),
            description: metadata.description.clone(),
        }],
        ..Default::default()
    }
}

/// Turn a library answer into domain data, keeping its partial successes.
fn settle(
    answer: Result<CommunityOperationResult, CommunityError>,
) -> Result<CommunityOperationResult, String> {
    match answer {
        Ok(result) => Ok(result),
        Err(CommunityError::ConfigurationFailed {
            created_jid,
            step,
            source,
        }) => {
            // The group exists. Retrying the creation would make a second
            // one, so the caller is told about the first and left to finish
            // the configuration if it wants to.
            let step = match step {
                CommunityConfigurationStep::SetDescription => CommunityWarningStep::SetDescription,
                _ => CommunityWarningStep::SetDescription,
            };
            warn!("community {created_jid} was created but a follow-up step failed: {source}");
            let jid = created_jid.to_string();
            Ok(CommunityOperationResult {
                groups: vec![CommunityGroupRef {
                    jid: jid.clone(),
                    subject: None,
                    description: None,
                }],
                warnings: vec![CommunityWarning {
                    step,
                    jid: Some(jid),
                    detail: None,
                }],
                ..Default::default()
            })
        }
        Err(CommunityError::MembershipApprovalRequired(jid)) => Ok(CommunityOperationResult {
            warnings: vec![CommunityWarning {
                step: CommunityWarningStep::JoinApprovalPending,
                jid: Some(jid.to_string()),
                detail: None,
            }],
            ..Default::default()
        }),
        Err(error) => Err(error.to_string()),
    }
}

#[cfg(all(test, not(target_family = "wasm")))]
mod tests {
    use super::*;
    use oxidezap_core::CommunityWarningStep;
    use whatsapp_rust::GroupError;

    fn jid(value: &str) -> Jid {
        value.parse().expect("a synthetic group address")
    }

    #[test]
    fn a_create_that_lost_its_description_reports_the_created_group() {
        let result = settle(Err(CommunityError::ConfigurationFailed {
            created_jid: jid("155555500001@g.us"),
            step: CommunityConfigurationStep::SetDescription,
            source: Box::new(GroupError::InvalidRequest(
                "synthetic description failure".into(),
            )),
        }))
        .expect("a partial creation is not a hard failure");

        assert_eq!(result.groups.len(), 1);
        assert_eq!(result.groups[0].jid, "155555500001@g.us");
        assert_eq!(result.groups[0].subject, None);
        assert_eq!(result.warnings.len(), 1);
        assert_eq!(
            result.warnings[0].step,
            CommunityWarningStep::SetDescription
        );
        assert_eq!(result.warnings[0].jid.as_deref(), Some("155555500001@g.us"));
        assert!(result.warnings[0].detail.is_none());
    }

    #[test]
    fn a_join_waiting_on_approval_is_a_warning_not_a_failure() {
        let result = settle(Err(CommunityError::MembershipApprovalRequired(jid(
            "155555500002@g.us",
        ))))
        .expect("approval pending is not a hard failure");

        assert!(result.groups.is_empty());
        assert_eq!(result.warnings.len(), 1);
        assert_eq!(
            result.warnings[0].step,
            CommunityWarningStep::JoinApprovalPending
        );
        assert_eq!(result.warnings[0].jid.as_deref(), Some("155555500002@g.us"));
    }

    #[test]
    fn a_partial_link_keeps_both_the_linked_and_refused_groups() {
        let result = settle(Ok(CommunityOperationResult {
            linked_jids: vec!["155555500001@g.us".into(), "155555500002@g.us".into()],
            failed_groups: vec![CommunitySubgroupFailure {
                jid: "155555500003@g.us".into(),
                code: 403,
            }],
            ..Default::default()
        }))
        .expect("a partial link is not a hard failure");

        assert_eq!(result.linked_jids.len(), 2);
        assert_eq!(result.failed_groups.len(), 1);
        assert_eq!(result.failed_groups[0].code, 403);
        assert_eq!(result.failed_groups[0].jid, "155555500003@g.us");
    }

    #[test]
    fn a_success_keeps_its_created_group() {
        let result = settle(Ok(CommunityOperationResult {
            groups: vec![CommunityGroupRef {
                jid: "155555500004@g.us".into(),
                subject: Some("Synthetic".into()),
                description: None,
            }],
            ..Default::default()
        }))
        .expect("success is Ok");

        assert_eq!(result.groups[0].jid, "155555500004@g.us");
        assert!(result.warnings.is_empty());
        assert!(result.failed_groups.is_empty());
    }

    #[test]
    fn a_hard_error_is_a_string_the_caller_can_show() {
        let error = settle(Err(CommunityError::InvalidRequest(
            "synthetic bad request".into(),
        )))
        .expect_err("a hard error is Err");
        assert!(error.contains("synthetic bad request"));
    }

    #[test]
    fn jids_are_checked_locally_before_any_request() {
        assert!(group_jid("155555500001@g.us", "group").is_ok());
        assert!(group_jid("155555500001@s.whatsapp.net", "group").is_err());
        assert!(group_jid("not-a-jid", "group").is_err());
        assert!(person_jid("155555500001@lid", "participant").is_ok());
        assert!(person_jid("not-a-jid", "participant").is_err());
    }
}
