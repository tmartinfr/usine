//! Links between Jira issues and the GitLab merge requests that mention them.

use crate::model::{Issue, MergeRequest};

/// Whether `mr` refers to the issue `key` in its source branch, title or
/// description, like the GitLab for Jira app does. Case is ignored.
pub fn mentions(mr: &MergeRequest, key: &str) -> bool {
    [&mr.source_branch, &mr.title, &mr.description].iter().any(|text| contains_key(text, key))
}

/// `key` as a whole: `PAY-12` is not found in `PAY-123` nor in `XPAY-12`.
fn contains_key(text: &str, key: &str) -> bool {
    // ASCII lowercase keeps byte offsets, so `text` can be indexed with them.
    let (haystack, key) = (text.to_ascii_lowercase(), key.to_ascii_lowercase());
    haystack.match_indices(&key).any(|(start, _)| {
        let before = haystack[..start].chars().next_back();
        let after = haystack[start + key.len()..].chars().next();
        !before.is_some_and(char::is_alphanumeric) && !after.is_some_and(|c| c.is_ascii_digit())
    })
}

/// Issues with their merge requests, as indexes into the slices given to [`link`].
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Links {
    /// One row per issue and merge request, in issue order then merge request
    /// order; an issue without merge request has one row with `None`.
    pub rows: Vec<(usize, Option<usize>)>,
    /// Merge requests mentioning none of the issues.
    pub others: Vec<usize>,
}

pub fn link(issues: &[Issue], mrs: &[MergeRequest]) -> Links {
    let mut links = Links::default();
    let mut linked = vec![false; mrs.len()];
    for (i, issue) in issues.iter().enumerate() {
        let start = links.rows.len();
        for (m, mr) in mrs.iter().enumerate() {
            if mentions(mr, &issue.key) {
                links.rows.push((i, Some(m)));
                linked[m] = true;
            }
        }
        if links.rows.len() == start {
            links.rows.push((i, None));
        }
    }
    links.others = (0..mrs.len()).filter(|&m| !linked[m]).collect();
    links
}

#[cfg(test)]
mod tests {
    use chrono::DateTime;

    use super::*;
    use crate::model::{Role, Source, StatusCategory};

    fn issue(key: &str) -> Issue {
        Issue {
            source: Source::Jira,
            key: key.into(),
            title: String::new(),
            status: "To Do".into(),
            status_category: StatusCategory::Todo,
            priority: None,
            project: "PAY".into(),
            kind: "Task".into(),
            description: String::new(),
            updated: DateTime::parse_from_rfc3339("2026-10-10T12:00:00Z").unwrap(),
            url: format!("https://example.atlassian.net/browse/{key}"),
        }
    }

    fn mr(iid: u64, branch: &str, title: &str, description: &str) -> MergeRequest {
        MergeRequest {
            source: Source::Gitlab,
            reference: format!("group/project!{iid}"),
            project: "group/project".into(),
            iid,
            title: title.into(),
            description: description.into(),
            draft: false,
            merge_status: "mergeable".into(),
            pipeline: None,
            source_branch: branch.into(),
            target_branch: "main".into(),
            author: "me".into(),
            role: Role::Author,
            updated: DateTime::parse_from_rfc3339("2026-10-10T12:00:00Z").unwrap(),
            url: format!("https://gitlab.com/group/project/-/merge_requests/{iid}"),
            worktree: None,
        }
    }

    #[test]
    fn finds_whole_keys_in_branch_title_or_description() {
        assert!(mentions(&mr(1, "pay-12-fix-retry", "", ""), "PAY-12"));
        assert!(mentions(&mr(1, "PAY-12_fix", "", ""), "PAY-12"));
        assert!(mentions(&mr(1, "feat", "[PAY-12] Fix retry", ""), "PAY-12"));
        assert!(mentions(&mr(1, "feat", "Fix", "Closes PAY-12."), "PAY-12"));
        assert!(mentions(&mr(1, "feat", "PAY-123 and PAY-12", ""), "PAY-12"));

        assert!(!mentions(&mr(1, "PAY-123-fix", "", ""), "PAY-12"));
        assert!(!mentions(&mr(1, "XPAY-12", "", ""), "PAY-12"));
        assert!(!mentions(&mr(1, "feat", "Fix", "See OPS-12"), "PAY-12"));
    }

    #[test]
    fn links_issues_to_their_merge_requests() {
        let issues = [issue("PAY-1"), issue("PAY-2"), issue("PAY-3")];
        let mrs = [
            mr(10, "PAY-1-a", "", ""),
            mr(11, "other", "", ""),
            mr(12, "PAY-3-b", "", "Also fixes PAY-1"),
            mr(13, "PAY-1-c", "", ""),
        ];

        let links = link(&issues, &mrs);

        assert_eq!(
            links.rows,
            [(0, Some(0)), (0, Some(2)), (0, Some(3)), (1, None), (2, Some(2))]
        );
        assert_eq!(links.others, [1]);
    }
}
