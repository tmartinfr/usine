//! What changed between two fetches, worded for a notification.

use crate::connectors::claude::Session;
use crate::model::{Issue, MergeRequest, Role};

/// New issues, status changes and issues no longer listed.
pub fn issue_changes(old: &[Issue], new: &[Issue]) -> Vec<String> {
    let mut changes = Vec::new();
    for issue in new {
        match old.iter().find(|o| o.key == issue.key) {
            None => changes.push(format!("New issue {}: {}", issue.key, issue.title)),
            Some(o) if o.status != issue.status => {
                changes.push(format!("{}: {} -> {}", issue.key, o.status, issue.status))
            }
            Some(_) => {}
        }
    }
    for o in old.iter().filter(|o| !new.iter().any(|i| i.key == o.key)) {
        changes.push(format!("{} is no longer listed", o.key));
    }
    changes
}

/// New MRs to review, merge status and pipeline changes (pipelines of my MRs
/// only), and MRs no longer listed (merged, closed or approved).
pub fn mr_changes(old: &[MergeRequest], new: &[MergeRequest]) -> Vec<String> {
    let mut changes = Vec::new();
    for mr in new {
        let Some(o) = old.iter().find(|o| o.reference == mr.reference) else {
            if mr.role == Role::Reviewer {
                changes.push(format!("New MR to review {}: {}", mr.reference, mr.title));
            }
            continue;
        };
        // GitLab reports these while it recomputes the status: not a real change.
        let transient = |s: &str| matches!(s, "unchecked" | "checking");
        if o.merge_status != mr.merge_status
            && !transient(&o.merge_status)
            && !transient(&mr.merge_status)
        {
            changes.push(format!("{}: {} -> {}", mr.reference, o.merge_status, mr.merge_status));
        }
        if mr.role == Role::Author && o.pipeline != mr.pipeline {
            let status = |p: &Option<String>| p.clone().unwrap_or_else(|| "none".into());
            changes.push(format!(
                "{}: pipeline {} -> {}",
                mr.reference,
                status(&o.pipeline),
                status(&mr.pipeline)
            ));
        }
    }
    for o in old.iter().filter(|o| !new.iter().any(|m| m.reference == o.reference)) {
        changes.push(format!("{} is no longer listed", o.reference));
    }
    changes
}

/// Reviews that were running in `old` and are finished in `new`.
pub fn finished_reviews(old: &[Session], new: &[Session]) -> Vec<String> {
    new.iter()
        .filter(|s| s.finished && old.iter().any(|o| o.name == s.name && !o.finished))
        .map(|s| format!("Review of {} finished", s.subject))
        .collect()
}

#[cfg(test)]
mod tests {
    use chrono::DateTime;

    use super::*;
    use crate::model::{Source, StatusCategory};

    fn issue(key: &str, status: &str) -> Issue {
        Issue {
            source: Source::Jira,
            key: key.into(),
            title: format!("Summary of {key}"),
            status: status.into(),
            status_category: StatusCategory::Todo,
            priority: None,
            project: "PAY".into(),
            kind: "Task".into(),
            updated: DateTime::parse_from_rfc3339("2026-10-10T12:00:00Z").unwrap(),
            url: String::new(),
        }
    }

    fn mr(iid: u32, role: Role, merge_status: &str, pipeline: Option<&str>) -> MergeRequest {
        MergeRequest {
            source: Source::Gitlab,
            reference: format!("group/project!{iid}"),
            project: "group/project".into(),
            iid: iid.into(),
            title: format!("MR {iid}"),
            description: String::new(),
            draft: false,
            merge_status: merge_status.into(),
            pipeline: pipeline.map(Into::into),
            source_branch: "feat".into(),
            target_branch: "main".into(),
            author: "me".into(),
            role,
            updated: DateTime::parse_from_rfc3339("2026-10-10T12:00:00Z").unwrap(),
            url: String::new(),
        }
    }

    fn session(iid: u32, finished: bool) -> Session {
        Session { name: format!("review-{iid}"), subject: format!("group/project!{iid}"), finished }
    }

    #[test]
    fn issue_changes_report_new_status_and_gone() {
        let old = [issue("PAY-1", "To Do"), issue("PAY-2", "To Do"), issue("PAY-3", "To Do")];
        let new = [issue("PAY-1", "To Do"), issue("PAY-2", "In Progress"), issue("PAY-4", "To Do")];

        assert_eq!(
            issue_changes(&old, &new),
            [
                "PAY-2: To Do -> In Progress",
                "New issue PAY-4: Summary of PAY-4",
                "PAY-3 is no longer listed",
            ]
        );
        assert!(issue_changes(&new, &new).is_empty());
    }

    #[test]
    fn mr_changes_report_reviews_statuses_pipelines_and_gone() {
        let old = [
            mr(1, Role::Author, "not_approved", Some("running")),
            mr(2, Role::Reviewer, "mergeable", Some("running")),
            mr(3, Role::Author, "mergeable", None),
            mr(4, Role::Author, "mergeable", None),
        ];
        let new = [
            mr(1, Role::Author, "mergeable", Some("failed")),
            // Pipelines of MRs I review are not reported.
            mr(2, Role::Reviewer, "mergeable", Some("success")),
            mr(3, Role::Author, "unchecked", None),
            mr(5, Role::Reviewer, "mergeable", None),
            mr(6, Role::Author, "mergeable", None),
        ];

        assert_eq!(
            mr_changes(&old, &new),
            [
                "group/project!1: not_approved -> mergeable",
                "group/project!1: pipeline running -> failed",
                "New MR to review group/project!5: MR 5",
                "group/project!4 is no longer listed",
            ]
        );
    }

    #[test]
    fn finished_reviews_only_report_transitions() {
        let old = [session(1, false), session(2, true), session(3, false)];
        let new = [session(1, true), session(2, true), session(3, false), session(4, true)];

        assert_eq!(finished_reviews(&old, &new), ["Review of group/project!1 finished"]);
    }
}
