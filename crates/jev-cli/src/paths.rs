//! Path comparisons shared by the commands that write files.
//!
//! `jev map` and `jev eval` both take paths to read from and paths to write to, and both
//! have to refuse the mistake of naming one file twice. The comparison lives here so
//! there is one definition of "the same file" rather than two that can disagree about
//! `./out.jsonl` versus `out.jsonl`.

use std::path::Path;

/// Whether two paths the user wrote name the same file.
///
/// Best effort, and deliberately so. A whole-path `==` misses `./out.jsonl` against
/// `out.jsonl`, which is the *likely* way to write the same file twice by accident, so
/// the directory part is resolved where it can be — the parent usually exists even when
/// the file does not — and the file names are compared directly.
///
/// This is a guard against a mistake, not a security control. Two different paths can
/// still reach one file through a symlink or a hard link, and `jev` does not chase
/// those: resolving a symlink the user deliberately created would be the wrong answer to
/// a different question. What it must never do is *refuse* two paths that are genuinely
/// different, so every step falls back to the unresolved form rather than guessing.
pub(crate) fn same_file(left: &Path, right: &Path) -> bool {
    if left == right {
        return true;
    }
    let resolved = |path: &Path| {
        let parent = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty());
        let directory = match parent {
            Some(parent) => parent.canonicalize().ok(),
            // A bare file name means the working directory.
            None => std::env::current_dir().ok(),
        };
        directory.map(|directory| (directory, path.file_name().map(std::ffi::OsStr::to_owned)))
    };
    match (resolved(left), resolved(right)) {
        // Both names must be present: two paths that each end in `..` are not
        // comparable this way, and saying "same file" about them would be a guess.
        (Some((left_dir, Some(left_name))), Some((right_dir, Some(right_name)))) => {
            left_dir == right_dir && left_name == right_name
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_path_is_the_same_file_as_itself_however_it_is_spelled() {
        let directory = tempfile::tempdir().expect("a temporary directory");
        let plain = directory.path().join("out.jsonl");
        let dotted = directory.path().join(".").join("out.jsonl");
        assert!(same_file(&plain, &plain));
        assert!(same_file(&plain, &dotted));
    }

    #[test]
    fn two_genuinely_different_paths_are_not_the_same_file() {
        let directory = tempfile::tempdir().expect("a temporary directory");
        assert!(!same_file(
            &directory.path().join("out.jsonl"),
            &directory.path().join("review.jsonl")
        ));
    }

    #[test]
    fn an_uncomparable_pair_is_reported_as_different_rather_than_guessed() {
        // Neither path has a file name to compare, so there is nothing to decide on.
        // Reported as different, because the one failure mode this guard must never have
        // is refusing two paths that are genuinely separate files.
        assert!(!same_file(Path::new("a/.."), Path::new("b/..")));
    }

    #[test]
    fn a_path_spelled_identically_is_the_same_file_without_touching_the_filesystem() {
        // The literal equality check comes first, so a path that does not exist -- and
        // therefore cannot be canonicalized -- is still caught when it is named twice.
        let path = Path::new("/nonexistent/jev/out.jsonl");
        assert!(same_file(path, path));
    }
}
