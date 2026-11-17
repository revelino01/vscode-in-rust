//! Git SCM repository model and actions.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum GitStatusKind {
    Untracked,
    Modified,
    Added,
    Deleted,
    Renamed,
    Conflicted,
}

impl GitStatusKind {
    pub fn badge_str(&self) -> &'static str {
        match self {
            Self::Untracked => "U",
            Self::Modified => "M",
            Self::Added => "A",
            Self::Deleted => "D",
            Self::Renamed => "R",
            Self::Conflicted => "C",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GitStatusEntry {
    pub path: PathBuf,
    pub status: GitStatusKind,
    pub is_staged: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BranchInfo {
    pub name: String,
    pub is_current: bool,
    pub upstream: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CommitInfo {
    pub hash: String,
    pub author: String,
    pub message: String,
    pub timestamp: u64,
}

/// SCM repository tracking working tree, staged index, branches, and commit operations.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScmRepository {
    pub root: PathBuf,
    pub current_branch: BranchInfo,
    pub staged: Vec<GitStatusEntry>,
    pub changes: Vec<GitStatusEntry>,
    pub untracked: Vec<GitStatusEntry>,
    pub commits: Vec<CommitInfo>,
    pub file_contents: HashMap<PathBuf, (String, String)>, // (HEAD content, working tree content)
}

impl Default for ScmRepository {
    fn default() -> Self {
        Self::new(PathBuf::from("."))
    }
}

impl ScmRepository {
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            current_branch: BranchInfo {
                name: "main".to_string(),
                is_current: true,
                upstream: Some("origin/main".to_string()),
            },
            staged: Vec::new(),
            changes: Vec::new(),
            untracked: Vec::new(),
            commits: Vec::new(),
            file_contents: HashMap::new(),
        }
    }

    pub fn set_file_diff(&mut self, path: impl AsRef<Path>, original: &str, modified: &str) {
        let p = path.as_ref().to_path_buf();
        self.file_contents.insert(p, (original.to_string(), modified.to_string()));
    }

    pub fn get_file_diff(&self, path: impl AsRef<Path>) -> Option<(&str, &str)> {
        self.file_contents
            .get(path.as_ref())
            .map(|(o, m)| (o.as_str(), m.as_str()))
    }

    pub fn stage_file(&mut self, path: impl AsRef<Path>) -> bool {
        let p = path.as_ref();

        // Check if in changes
        if let Some(pos) = self.changes.iter().position(|e| e.path == p) {
            let mut entry = self.changes.remove(pos);
            entry.is_staged = true;
            self.staged.retain(|e| e.path != p);
            self.staged.push(entry);
            return true;
        }

        // Check if in untracked
        if let Some(pos) = self.untracked.iter().position(|e| e.path == p) {
            let mut entry = self.untracked.remove(pos);
            entry.is_staged = true;
            entry.status = GitStatusKind::Added;
            self.staged.retain(|e| e.path != p);
            self.staged.push(entry);
            return true;
        }

        false
    }

    pub fn unstage_file(&mut self, path: impl AsRef<Path>) -> bool {
        let p = path.as_ref();
        if let Some(pos) = self.staged.iter().position(|e| e.path == p) {
            let mut entry = self.staged.remove(pos);
            entry.is_staged = false;
            if entry.status == GitStatusKind::Added {
                entry.status = GitStatusKind::Untracked;
                self.untracked.retain(|e| e.path != p);
                self.untracked.push(entry);
            } else {
                self.changes.retain(|e| e.path != p);
                self.changes.push(entry);
            }
            return true;
        }
        false
    }

    pub fn stage_all(&mut self) {
        let all_changes = std::mem::take(&mut self.changes);
        for mut c in all_changes {
            c.is_staged = true;
            self.staged.push(c);
        }

        let all_untracked = std::mem::take(&mut self.untracked);
        for mut u in all_untracked {
            u.is_staged = true;
            u.status = GitStatusKind::Added;
            self.staged.push(u);
        }
    }

    pub fn unstage_all(&mut self) {
        let all_staged = std::mem::take(&mut self.staged);
        for mut s in all_staged {
            s.is_staged = false;
            if s.status == GitStatusKind::Added {
                s.status = GitStatusKind::Untracked;
                self.untracked.push(s);
            } else {
                self.changes.push(s);
            }
        }
    }

    pub fn discard_file(&mut self, path: impl AsRef<Path>) -> bool {
        let p = path.as_ref();
        if let Some(pos) = self.changes.iter().position(|e| e.path == p) {
            self.changes.remove(pos);
            // Revert modified content back to original if present
            if let Some((orig, modif)) = self.file_contents.get_mut(p) {
                *modif = orig.clone();
            }
            return true;
        }
        false
    }

    pub fn commit(&mut self, message: &str) -> Result<CommitInfo, String> {
        let trimmed = message.trim();
        if trimmed.is_empty() {
            return Err("Cannot commit with empty commit message".to_string());
        }

        if self.staged.is_empty() {
            return Err("No changes staged to commit".to_string());
        }

        // Generate synthetic commit hash for in-memory tracker
        let hash = format!("{:07x}", self.commits.len() + 1);
        let commit = CommitInfo {
            hash: hash.clone(),
            author: "RustCode Developer <dev@rustcode.local>".to_string(),
            message: trimmed.to_string(),
            timestamp: 1700000000 + self.commits.len() as u64 * 60,
        };

        // Update original contents to match committed state for staged files
        for entry in &self.staged {
            if let Some((orig, modif)) = self.file_contents.get_mut(&entry.path) {
                *orig = modif.clone();
            }
        }

        self.staged.clear();
        self.commits.push(commit.clone());
        Ok(commit)
    }

    /// Sample repository with pre-configured staged, changed, and untracked files for tests/previews.
    pub fn sample_repository() -> Self {
        let mut repo = Self::new(PathBuf::from("/workspace/sample-project"));

        let file_rs = PathBuf::from("src/main.rs");
        repo.changes.push(GitStatusEntry {
            path: file_rs.clone(),
            status: GitStatusKind::Modified,
            is_staged: false,
        });
        repo.set_file_diff(
            &file_rs,
            "fn main() {\n    println!(\"Hello\");\n}\n",
            "fn main() {\n    let name = \"RustCode\";\n    println!(\"Hello, {}\", name);\n}\n",
        );

        let cargo_toml = PathBuf::from("Cargo.toml");
        repo.staged.push(GitStatusEntry {
            path: cargo_toml.clone(),
            status: GitStatusKind::Modified,
            is_staged: true,
        });
        repo.set_file_diff(
            &cargo_toml,
            "[package]\nname = \"demo\"\nversion = \"0.1.0\"\n",
            "[package]\nname = \"demo\"\nversion = \"0.2.0\"\n",
        );

        let readme = PathBuf::from("README.md");
        repo.untracked.push(GitStatusEntry {
            path: readme.clone(),
            status: GitStatusKind::Untracked,
            is_staged: false,
        });
        repo.set_file_diff(&readme, "", "# Demo Project\n\nBuilt with RustCode.\n");

        repo
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_scm_repository_stage_unstage_commit() {
        let mut repo = ScmRepository::sample_repository();
        assert_eq!(repo.staged.len(), 1);
        assert_eq!(repo.changes.len(), 1);
        assert_eq!(repo.untracked.len(), 1);

        // Stage src/main.rs from changes
        assert!(repo.stage_file("src/main.rs"));
        assert_eq!(repo.staged.len(), 2);
        assert_eq!(repo.changes.len(), 0);

        // Stage README.md from untracked
        assert!(repo.stage_file("README.md"));
        assert_eq!(repo.staged.len(), 3);
        assert_eq!(repo.untracked.len(), 0);

        // Unstage README.md -> returns to untracked
        assert!(repo.unstage_file("README.md"));
        assert_eq!(repo.staged.len(), 2);
        assert_eq!(repo.untracked.len(), 1);
        assert_eq!(repo.untracked[0].status, GitStatusKind::Untracked);

        // Commit fails with empty message
        assert!(repo.commit("   ").is_err());

        // Commit succeeds with message
        let commit = repo.commit("feat: update Cargo.toml and src/main.rs").unwrap();
        assert_eq!(commit.message, "feat: update Cargo.toml and src/main.rs");
        assert!(repo.staged.is_empty());
        assert_eq!(repo.commits.len(), 1);
    }
}
