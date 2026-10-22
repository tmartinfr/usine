//! Columns of the panes and how each one sorts.

use std::cmp::Ordering;

use usine::model::{Issue, MergeRequest};

pub const ISSUE_COLUMNS: [&str; 5] = ["KEY", "STATUS", "PRIORITY", "UPDATED", "SUMMARY"];
pub const MR_COLUMNS: [&str; 7] = ["R", "ROLE", "MR", "STATUS", "PIPELINE", "UPDATED", "TITLE"];

/// Jira issues by status, then most recently updated first (the source order).
pub const ISSUE_DEFAULT_SORT: Sort = Sort { column: 1, descending: false };
/// Authored merge requests first, then most recently updated first.
pub const MR_DEFAULT_SORT: Sort = Sort { column: 1, descending: false };

/// Column a pane is sorted on, as an index into its columns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sort {
    pub column: usize,
    pub descending: bool,
}

impl Sort {
    pub fn apply(self, ordering: Ordering) -> Ordering {
        if self.descending { ordering.reverse() } else { ordering }
    }
}

/// Review state of a merge request, in ascending sort order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ReviewState {
    Running,
    Starting,
    Finished,
    Absent,
}

/// Compares on a column of `ISSUE_COLUMNS`.
pub fn compare_issues(a: &Issue, b: &Issue, column: usize) -> Ordering {
    match column {
        0 => key_parts(&a.key).cmp(&key_parts(&b.key)),
        1 => (a.status_category, a.status.to_lowercase())
            .cmp(&(b.status_category, b.status.to_lowercase())),
        2 => priority_rank(a.priority.as_deref()).cmp(&priority_rank(b.priority.as_deref())),
        3 => a.updated.cmp(&b.updated),
        _ => a.title.to_lowercase().cmp(&b.title.to_lowercase()),
    }
}

/// Compares on a column of `MR_COLUMNS`.
pub fn compare_merge_requests(
    a: &MergeRequest,
    b: &MergeRequest,
    column: usize,
    review: impl Fn(&MergeRequest) -> ReviewState,
) -> Ordering {
    match column {
        0 => review(a).cmp(&review(b)),
        1 => a.role.cmp(&b.role),
        2 => (a.project.to_lowercase(), a.iid).cmp(&(b.project.to_lowercase(), b.iid)),
        3 => (merge_readiness(&a.merge_status), &a.merge_status)
            .cmp(&(merge_readiness(&b.merge_status), &b.merge_status)),
        4 => (pipeline_rank(a.pipeline.as_deref()), &a.pipeline)
            .cmp(&(pipeline_rank(b.pipeline.as_deref()), &b.pipeline)),
        5 => a.updated.cmp(&b.updated),
        _ => a.title.to_lowercase().cmp(&b.title.to_lowercase()),
    }
}

/// `PAY-123` sorts as (`PAY`, 123), so that `PAY-9` comes before `PAY-10`.
fn key_parts(key: &str) -> (&str, u64) {
    match key.rsplit_once('-') {
        Some((project, number)) => (project, number.parse().unwrap_or(0)),
        None => (key, 0),
    }
}

/// Jira default and common custom priority names, most important first;
/// unknown names and no priority come last.
pub fn priority_rank(priority: Option<&str>) -> u8 {
    let Some(priority) = priority else { return 6 };
    match priority.to_lowercase().as_str() {
        "highest" | "blocker" | "critical" | "urgent" => 0,
        "high" | "major" => 1,
        "medium" => 2,
        "low" | "minor" => 3,
        "lowest" | "trivial" => 4,
        _ => 5,
    }
}

/// GitLab `detailed_merge_status` values: 0 ready, 1 in progress, 2 blocked, 3 draft.
pub fn merge_readiness(status: &str) -> u8 {
    match status {
        "mergeable" => 0,
        "draft_status" => 3,
        "unchecked" | "checking" | "preparing" | "approvals_syncing" | "ci_still_running"
        | "not_approved" => 1,
        _ => 2,
    }
}

/// GitLab pipeline statuses: 0 passed, 1 in progress or waiting, 2 failed,
/// 3 canceled or skipped, 4 no pipeline.
pub fn pipeline_rank(status: Option<&str>) -> u8 {
    let Some(status) = status else { return 4 };
    match status {
        "success" => 0,
        "created" | "waiting_for_resource" | "preparing" | "pending" | "running" | "scheduled"
        | "manual" => 1,
        "failed" => 2,
        _ => 3,
    }
}

#[cfg(test)]
mod tests {
    use chrono::DateTime;
    use usine::model::Role;

    use super::*;
    use crate::tui::app::tests::{issue, mr};

    fn sorted_keys(mut issues: Vec<Issue>, column: usize) -> Vec<String> {
        issues.sort_by(|a, b| compare_issues(a, b, column));
        issues.into_iter().map(|i| i.key).collect()
    }

    #[test]
    fn issue_keys_sort_by_project_then_number() {
        let issues = vec![issue("PAY-10"), issue("OPS-7"), issue("PAY-9")];
        assert_eq!(sorted_keys(issues, 0), ["OPS-7", "PAY-9", "PAY-10"]);
    }

    #[test]
    fn issue_priorities_sort_most_important_first() {
        let with_priority = |key: &str, priority: Option<&str>| Issue {
            priority: priority.map(Into::into),
            ..issue(key)
        };
        let issues = vec![
            with_priority("PAY-1", None),
            with_priority("PAY-2", Some("Low")),
            with_priority("PAY-3", Some("Highest")),
            with_priority("PAY-4", Some("Medium")),
        ];
        assert_eq!(sorted_keys(issues, 2), ["PAY-3", "PAY-4", "PAY-2", "PAY-1"]);
    }

    #[test]
    fn merge_requests_sort_by_readiness_and_date() {
        let mut blocked = mr(1);
        blocked.merge_status = "conflict".into();
        let mut waiting = mr(2);
        waiting.merge_status = "not_approved".into();
        waiting.updated = DateTime::parse_from_rfc3339("2026-10-01T12:00:00Z").unwrap();
        let ready = mr(3);
        let mut mrs = vec![blocked, waiting, ready];
        let iids = |mrs: &[MergeRequest]| mrs.iter().map(|m| m.iid).collect::<Vec<_>>();

        mrs.sort_by(|a, b| compare_merge_requests(a, b, 3, |_| ReviewState::Absent));
        assert_eq!(iids(&mrs), [3, 2, 1]);

        let sort = Sort { column: 5, descending: true };
        mrs.sort_by(|a, b| sort.apply(compare_merge_requests(a, b, 5, |_| ReviewState::Absent)));
        assert_eq!(iids(&mrs), [3, 1, 2]);
    }

    #[test]
    fn merge_requests_sort_by_pipeline() {
        let with_pipeline = |iid, pipeline: Option<&str>| MergeRequest {
            pipeline: pipeline.map(Into::into),
            ..mr(iid)
        };
        let mut mrs = [
            with_pipeline(1, None),
            with_pipeline(2, Some("failed")),
            with_pipeline(3, Some("canceled")),
            with_pipeline(4, Some("running")),
            with_pipeline(5, Some("success")),
        ];
        mrs.sort_by(|a, b| compare_merge_requests(a, b, 4, |_| ReviewState::Absent));
        let iids: Vec<u64> = mrs.iter().map(|m| m.iid).collect();
        assert_eq!(iids, [5, 4, 2, 3, 1]);
    }

    #[test]
    fn merge_requests_sort_by_role() {
        let mut mrs = [MergeRequest { role: Role::Reviewer, ..mr(1) }, mr(2)];
        mrs.sort_by(|a, b| compare_merge_requests(a, b, 1, |_| ReviewState::Absent));
        assert_eq!(mrs.map(|m| m.iid), [2, 1]);
    }
}
