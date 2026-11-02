//! Git branch display values: pure parsing over file contents and paths.
//!
//! The shell owns every filesystem read; this module only interprets
//! what those reads return, so it performs no I/O and owns no
//! framework types. Display only: nothing here ever switches branches.

/// Parse `.git/HEAD` content into a display branch name.
///
/// - `ref: refs/heads/<name>` yields `<name>` (slashes preserved),
/// - a bare hex hash (detached HEAD) yields its first 7 characters,
/// - anything else (empty input, a non-heads ref, garbage) yields `None`.
///
/// Only the first line matters; trailing newlines are ignored.
pub fn parse_head(content: &str) -> Option<String> {
    let line = content.lines().next().unwrap_or("").trim();
    if let Some(target) = line.strip_prefix("ref:") {
        let target = target.trim();
        let name = target.strip_prefix("refs/heads/")?;
        if name.is_empty() || name.chars().any(char::is_whitespace) {
            return None;
        }
        return Some(name.to_string());
    }
    if line.len() >= 7 && line.chars().all(|got| got.is_ascii_hexdigit()) {
        return Some(line[..7].to_string());
    }
    None
}

/// Parse the first line of a `.git` pointer file (`gitdir: <path>`).
///
/// Worktree checkouts keep a plain file at `<workspace>/.git` naming
/// the real git dir; this returns that raw path, relative or absolute.
/// Anything else (empty input, no `gitdir:` prefix, blank path)
/// yields `None`.
pub fn parse_gitdir_pointer(content: &str) -> Option<&str> {
    let line = content.lines().next().unwrap_or("").trim();
    let path = line.strip_prefix("gitdir:")?.trim();
    if path.is_empty() { None } else { Some(path) }
}

/// Resolve a gitdir pointer against its workspace: absolute pointers
/// stand alone, relative ones join the workspace with `/`.
/// Pure string join; the filesystem call that reads through it decides
/// whether the result exists.
pub fn resolve_git_dir(workspace: &str, pointer: &str) -> String {
    if pointer.starts_with('/') {
        pointer.to_string()
    } else {
        format!("{}/{}", workspace.trim_end_matches('/'), pointer)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn head_ref_yields_branch() {
        assert_eq!(
            parse_head("ref: refs/heads/main\n"),
            Some(String::from("main"))
        );
    }

    #[test]
    fn head_ref_keeps_slashes() {
        assert_eq!(
            parse_head("ref: refs/heads/feature/login-retry"),
            Some(String::from("feature/login-retry"))
        );
    }

    #[test]
    fn head_detached_hash_yields_short_hash() {
        assert_eq!(
            parse_head("9d3a1f2c4b5e6a7890abcdef1234567890abcdef1\n"),
            Some(String::from("9d3a1f2"))
        );
    }

    #[test]
    fn head_non_heads_ref_is_none() {
        assert_eq!(parse_head("ref: refs/tags/v1.2.3\n"), None);
    }

    #[test]
    fn head_empty_and_garbage_are_none() {
        assert_eq!(parse_head(""), None);
        assert_eq!(parse_head("\n"), None);
        assert_eq!(parse_head("not a head at all"), None);
        assert_eq!(parse_head("ref: refs/heads/\n"), None);
    }

    #[test]
    fn gitdir_pointer_parses_first_line() {
        assert_eq!(
            parse_gitdir_pointer("gitdir: /work/trees/main/.git\n"),
            Some("/work/trees/main/.git")
        );
        assert_eq!(
            parse_gitdir_pointer("gitdir: ../.git/worktrees/fix\n"),
            Some("../.git/worktrees/fix")
        );
    }

    #[test]
    fn gitdir_pointer_rejects_non_pointer_content() {
        assert_eq!(parse_gitdir_pointer(""), None);
        assert_eq!(parse_gitdir_pointer("dir: somewhere"), None);
        assert_eq!(parse_gitdir_pointer("gitdir:   \n"), None);
    }

    #[test]
    fn git_dir_resolution_keeps_absolute_joins_relative() {
        assert_eq!(
            resolve_git_dir("/work/checkout", "/elsewhere/.git"),
            String::from("/elsewhere/.git")
        );
        assert_eq!(
            resolve_git_dir("/work/checkout", "../.git/worktrees/fix"),
            String::from("/work/checkout/../.git/worktrees/fix")
        );
        assert_eq!(
            resolve_git_dir("/work/checkout/", ".git"),
            String::from("/work/checkout/.git")
        );
    }
}
