/// Splits a git remote URL into `(host, project path)`.
///
/// Supports `git@host:group/project.git`, `ssh://git@host:22/group/project.git`
/// and `https://host/group/project` forms.
pub fn parse_remote(url: &str) -> Option<(String, String)> {
    let (host, path) = match url.split_once("://") {
        Some((_scheme, rest)) => rest.split_once('/')?,
        None => url.split_once(':')?,
    };
    let host = host.rsplit_once('@').map_or(host, |(_, h)| h);
    let host = host.split_once(':').map_or(host, |(h, _port)| h);
    let path = path.trim_matches('/');
    let path = path.strip_suffix(".git").unwrap_or(path);
    if host.is_empty() || path.is_empty() {
        return None;
    }
    Some((host.to_lowercase(), path.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parsed(url: &str) -> Option<(String, String)> {
        parse_remote(url)
    }

    #[test]
    fn parses_common_remote_forms() {
        let expected = Some(("gitlab.com".to_string(), "paylead/infra/nomad".to_string()));
        assert_eq!(parsed("git@gitlab.com:paylead/infra/nomad.git"), expected);
        assert_eq!(parsed("git@gitlab.com:paylead/infra/nomad"), expected);
        assert_eq!(parsed("ssh://git@gitlab.com:22/paylead/infra/nomad.git"), expected);
        assert_eq!(parsed("https://gitlab.com/paylead/infra/nomad.git"), expected);
        assert_eq!(parsed("https://oauth2:token@GitLab.com/paylead/infra/nomad/"), expected);
        assert_eq!(parsed("https://gitlab.com"), None);
    }

    #[test]
    fn rejects_garbage() {
        assert_eq!(parsed(""), None);
        assert_eq!(parsed("/local/path/repo"), None);
    }
}
