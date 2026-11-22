//! Git SCM repository model, `GitBackend` trait, `InMemoryGitBackend`, and real `GitCliBackend`.

use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use thiserror::Error;

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

/// Snapshot returned by `GitBackend::status`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GitStatusSnapshot {
    pub branch: BranchInfo,
    pub head_oid: Option<String>,
    pub ahead: u32,
    pub behind: u32,
    pub staged: Vec<GitStatusEntry>,
    pub changes: Vec<GitStatusEntry>,
    pub untracked: Vec<GitStatusEntry>,
}

/// Errors produced by `GitBackend` operations.
#[derive(Debug, Error)]
pub enum GitError {
    #[error("git executable not found in PATH")]
    GitNotFound,
    #[error("not a git repository: {0}")]
    NotAGitRepository(PathBuf),
    #[error("cannot commit with empty commit message")]
    EmptyCommitMessage,
    #[error("no changes staged to commit")]
    NothingToCommit,
    #[error("git command `{command}` failed (exit {code:?}): {stderr}")]
    CommandFailed {
        command: String,
        code: Option<i32>,
        stderr: String,
    },
    #[error("failed to parse git output: {0}")]
    ParseError(String),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

/// Backend abstraction for Git SCM operations (real `git` CLI or deterministic in-memory mock).
pub trait GitBackend: Send + Sync {
    fn root(&self) -> &Path;
    fn is_git_repo(&self) -> bool;
    fn status(&self) -> Result<GitStatusSnapshot, GitError>;
    fn current_branch(&self) -> Result<BranchInfo, GitError>;
    fn diff(&self, path: &Path, staged: bool) -> Result<String, GitError>;
    fn show_head(&self, path: &Path) -> Result<Option<String>, GitError>;
    fn stage(&self, path: &Path) -> Result<(), GitError>;
    fn unstage(&self, path: &Path) -> Result<(), GitError>;
    fn stage_all(&self) -> Result<(), GitError>;
    fn unstage_all(&self) -> Result<(), GitError>;
    fn discard(&self, path: &Path) -> Result<(), GitError>;
    fn commit(&self, message: &str) -> Result<CommitInfo, GitError>;
    fn log(&self, limit: usize) -> Result<Vec<CommitInfo>, GitError>;
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

    /// Creates an empty SCM repository model with no fabricated branch/upstream or sample entries.
    pub fn empty(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            current_branch: BranchInfo {
                name: String::new(),
                is_current: true,
                upstream: None,
            },
            staged: Vec::new(),
            changes: Vec::new(),
            untracked: Vec::new(),
            commits: Vec::new(),
            file_contents: HashMap::new(),
        }
    }

    /// Loads live repository state from disk using `GitCliBackend`.
    pub fn from_git_cli(root: impl AsRef<Path>) -> Result<Self, GitError> {
        let backend = GitCliBackend::new(root.as_ref());
        Self::from_backend(&backend)
    }

    /// Loads repository state from any `GitBackend` implementation.
    pub fn from_backend(backend: &dyn GitBackend) -> Result<Self, GitError> {
        let mut repo = Self::empty(backend.root());
        repo.refresh_from_backend(backend)?;
        Ok(repo)
    }

    /// Refreshes this repository snapshot in-place from a `GitBackend`.
    pub fn refresh_from_backend(&mut self, backend: &dyn GitBackend) -> Result<(), GitError> {
        let snap = backend.status()?;
        self.current_branch = snap.branch;
        self.staged = snap.staged;
        self.changes = snap.changes;
        self.untracked = snap.untracked;
        self.commits = backend.log(20)?;
        self.file_contents.clear();

        const MAX_PRELOAD_FILES: usize = 100;
        const MAX_PRELOAD_BYTES: u64 = 512 * 1024;
        let mut seen = std::collections::HashSet::new();
        for entry in self.staged.iter().chain(self.changes.iter()) {
            if seen.len() >= MAX_PRELOAD_FILES {
                break;
            }
            if !seen.insert(entry.path.clone()) {
                continue;
            }
            let head_content = backend.show_head(&entry.path)?.unwrap_or_default();
            let full_path = backend.root().join(&entry.path);
            let work_content = match std::fs::metadata(&full_path) {
                Ok(meta) if meta.is_file() && meta.len() <= MAX_PRELOAD_BYTES => {
                    std::fs::read_to_string(&full_path).unwrap_or_default()
                }
                _ => String::new(),
            };
            self.file_contents
                .insert(entry.path.clone(), (head_content, work_content));
        }

        Ok(())
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

/// Deterministic in-memory `GitBackend` backed by `ScmRepository` for headless/unit tests.
pub struct InMemoryGitBackend {
    root: PathBuf,
    repo: RwLock<ScmRepository>,
}

impl InMemoryGitBackend {
    pub fn new(repo: ScmRepository) -> Self {
        let root = repo.root.clone();
        Self {
            root,
            repo: RwLock::new(repo),
        }
    }

    pub fn snapshot(&self) -> ScmRepository {
        self.repo.read().clone()
    }
}

impl GitBackend for InMemoryGitBackend {
    fn root(&self) -> &Path {
        &self.root
    }

    fn is_git_repo(&self) -> bool {
        true
    }

    fn status(&self) -> Result<GitStatusSnapshot, GitError> {
        let repo = self.repo.read();
        Ok(GitStatusSnapshot {
            branch: repo.current_branch.clone(),
            head_oid: repo.commits.last().map(|c| c.hash.clone()),
            ahead: 0,
            behind: 0,
            staged: repo.staged.clone(),
            changes: repo.changes.clone(),
            untracked: repo.untracked.clone(),
        })
    }

    fn current_branch(&self) -> Result<BranchInfo, GitError> {
        Ok(self.repo.read().current_branch.clone())
    }

    fn diff(&self, path: &Path, _staged: bool) -> Result<String, GitError> {
        let repo = self.repo.read();
        if let Some((orig, modif)) = repo.get_file_diff(path) {
            Ok(format!(
                "--- a/{}\n+++ b/{}\n- {}\n+ {}\n",
                path.display(),
                path.display(),
                orig,
                modif
            ))
        } else {
            Ok(String::new())
        }
    }

    fn show_head(&self, path: &Path) -> Result<Option<String>, GitError> {
        Ok(self
            .repo
            .read()
            .get_file_diff(path)
            .map(|(orig, _)| orig.to_string()))
    }

    fn stage(&self, path: &Path) -> Result<(), GitError> {
        self.repo.write().stage_file(path);
        Ok(())
    }

    fn unstage(&self, path: &Path) -> Result<(), GitError> {
        self.repo.write().unstage_file(path);
        Ok(())
    }

    fn stage_all(&self) -> Result<(), GitError> {
        self.repo.write().stage_all();
        Ok(())
    }

    fn unstage_all(&self) -> Result<(), GitError> {
        self.repo.write().unstage_all();
        Ok(())
    }

    fn discard(&self, path: &Path) -> Result<(), GitError> {
        self.repo.write().discard_file(path);
        Ok(())
    }

    fn commit(&self, message: &str) -> Result<CommitInfo, GitError> {
        let mut repo = self.repo.write();
        if message.trim().is_empty() {
            return Err(GitError::EmptyCommitMessage);
        }
        if repo.staged.is_empty() {
            return Err(GitError::NothingToCommit);
        }
        repo.commit(message).map_err(|e| GitError::CommandFailed {
            command: "commit".to_string(),
            code: Some(1),
            stderr: e,
        })
    }

    fn log(&self, limit: usize) -> Result<Vec<CommitInfo>, GitError> {
        let repo = self.repo.read();
        Ok(repo.commits.iter().rev().take(limit).cloned().collect())
    }
}

/// Real `git` CLI SCM backend executing `git -C <repo_root> ...` subprocesses.
#[derive(Debug, Clone)]
pub struct GitCliBackend {
    repo_root: PathBuf,
    git_binary: String,
}

impl GitCliBackend {
    pub fn new(repo_root: impl Into<PathBuf>) -> Self {
        Self {
            repo_root: repo_root.into(),
            git_binary: "git".to_string(),
        }
    }

    /// Creates a `GitCliBackend` with a custom `git` executable path (useful for testing).
    pub fn with_git_binary(repo_root: impl Into<PathBuf>, git_binary: impl Into<String>) -> Self {
        Self {
            repo_root: repo_root.into(),
            git_binary: git_binary.into(),
        }
    }

    fn build_command(&self, args: &[&str]) -> Command {
        let mut cmd = Command::new(&self.git_binary);
        cmd.arg("-C")
            .arg(&self.repo_root)
            .args(args)
            .env("GIT_TERMINAL_PROMPT", "0");

        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }

        cmd
    }

    fn run_git(&self, args: &[&str]) -> Result<std::process::Output, GitError> {
        if !self.repo_root.exists() {
            return Err(GitError::NotAGitRepository(self.repo_root.clone()));
        }

        let mut cmd = self.build_command(args);
        match cmd.output() {
            Ok(out) => Ok(out),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Err(GitError::GitNotFound),
            Err(e) => Err(GitError::Io(e)),
        }
    }

    fn run_git_checked(&self, args: &[&str]) -> Result<Vec<u8>, GitError> {
        let out = self.run_git(args)?;
        if out.status.success() {
            return Ok(out.stdout);
        }
        let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
        if is_not_a_git_repo_stderr(&stderr) {
            return Err(GitError::NotAGitRepository(self.repo_root.clone()));
        }
        Err(GitError::CommandFailed {
            command: format!("git {}", args.join(" ")),
            code: out.status.code(),
            stderr,
        })
    }

    fn has_head_commit(&self) -> Result<bool, GitError> {
        let out = self.run_git(&["rev-parse", "--verify", "HEAD"])?;
        if out.status.success() {
            return Ok(true);
        }
        let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
        if is_not_a_git_repo_stderr(&stderr) {
            return Err(GitError::NotAGitRepository(self.repo_root.clone()));
        }
        Ok(false)
    }

    fn to_repo_relative_path(&self, path: &Path) -> String {
        let rel = if path.is_absolute() {
            if let Ok(stripped) = path.strip_prefix(&self.repo_root) {
                stripped.to_path_buf()
            } else if let (Ok(canon_root), Ok(canon_path)) =
                (std::fs::canonicalize(&self.repo_root), std::fs::canonicalize(path))
            {
                canon_path
                    .strip_prefix(&canon_root)
                    .map(|p| p.to_path_buf())
                    .unwrap_or_else(|_| path.to_path_buf())
            } else {
                path.to_path_buf()
            }
        } else {
            path.to_path_buf()
        };

        let s = rel.to_string_lossy().replace('\\', "/");
        s.strip_prefix("./").unwrap_or(&s).to_string()
    }
}

impl GitBackend for GitCliBackend {
    fn root(&self) -> &Path {
        &self.repo_root
    }

    fn is_git_repo(&self) -> bool {
        if !self.repo_root.exists() {
            return false;
        }
        match self.run_git(&["rev-parse", "--is-inside-work-tree"]) {
            Ok(out) if out.status.success() => {
                String::from_utf8_lossy(&out.stdout).trim() == "true"
            }
            _ => false,
        }
    }

    fn status(&self) -> Result<GitStatusSnapshot, GitError> {
        let stdout = self.run_git_checked(&["status", "--porcelain=v2", "-b", "-z"])?;
        parse_porcelain_v2_z(&stdout)
    }

    fn current_branch(&self) -> Result<BranchInfo, GitError> {
        Ok(self.status()?.branch)
    }

    fn diff(&self, path: &Path, staged: bool) -> Result<String, GitError> {
        let rel = self.to_repo_relative_path(path);
        let args: Vec<&str> = if staged {
            vec!["diff", "--cached", "--", &rel]
        } else {
            vec!["diff", "--", &rel]
        };
        let stdout = self.run_git_checked(&args)?;
        Ok(String::from_utf8_lossy(&stdout).into_owned())
    }

    fn show_head(&self, path: &Path) -> Result<Option<String>, GitError> {
        let rel = self.to_repo_relative_path(path);
        let spec = format!("HEAD:{}", rel);
        let out = self.run_git(&["show", &spec])?;
        if out.status.success() {
            return Ok(Some(String::from_utf8_lossy(&out.stdout).into_owned()));
        }
        let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
        if is_not_a_git_repo_stderr(&stderr) {
            return Err(GitError::NotAGitRepository(self.repo_root.clone()));
        }
        if is_missing_head_object_stderr(&stderr) {
            return Ok(None);
        }
        Err(GitError::CommandFailed {
            command: format!("git show {}", spec),
            code: out.status.code(),
            stderr,
        })
    }

    fn stage(&self, path: &Path) -> Result<(), GitError> {
        let rel = self.to_repo_relative_path(path);
        self.run_git_checked(&["add", "--", &rel])?;
        Ok(())
    }

    fn unstage(&self, path: &Path) -> Result<(), GitError> {
        let rel = self.to_repo_relative_path(path);
        let out = self.run_git(&["restore", "--staged", "--", &rel])?;
        if out.status.success() {
            return Ok(());
        }
        let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
        if is_not_a_git_repo_stderr(&stderr) {
            return Err(GitError::NotAGitRepository(self.repo_root.clone()));
        }

        if self.has_head_commit()? {
            self.run_git_checked(&["reset", "HEAD", "--", &rel])?;
        } else {
            self.run_git_checked(&["rm", "--cached", "--", &rel])?;
        }
        Ok(())
    }

    fn stage_all(&self) -> Result<(), GitError> {
        self.run_git_checked(&["add", "-A"])?;
        Ok(())
    }

    fn unstage_all(&self) -> Result<(), GitError> {
        let out = self.run_git(&["restore", "--staged", "--", "."])?;
        if out.status.success() {
            return Ok(());
        }
        let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
        if is_not_a_git_repo_stderr(&stderr) {
            return Err(GitError::NotAGitRepository(self.repo_root.clone()));
        }

        if self.has_head_commit()? {
            self.run_git_checked(&["reset", "HEAD", "--", "."])?;
        } else {
            let rm_out = self.run_git(&["rm", "--cached", "-r", "-f", "--", "."])?;
            if !rm_out.status.success() {
                let rm_err = String::from_utf8_lossy(&rm_out.stderr).trim().to_string();
                if !rm_err.contains("did not match any files") {
                    return Err(GitError::CommandFailed {
                        command: "git rm --cached -r -f -- .".to_string(),
                        code: rm_out.status.code(),
                        stderr: rm_err,
                    });
                }
            }
        }
        Ok(())
    }

    fn discard(&self, path: &Path) -> Result<(), GitError> {
        let rel = self.to_repo_relative_path(path);
        let out = self.run_git(&["restore", "--worktree", "--", &rel])?;
        if out.status.success() {
            return Ok(());
        }
        self.run_git_checked(&["checkout", "--", &rel])?;
        Ok(())
    }

    fn commit(&self, message: &str) -> Result<CommitInfo, GitError> {
        let trimmed = message.trim();
        if trimmed.is_empty() {
            return Err(GitError::EmptyCommitMessage);
        }

        let out = self.run_git(&["commit", "-m", trimmed])?;
        if !out.status.success() {
            let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
            let stdout = String::from_utf8_lossy(&out.stdout).trim().to_string();
            let combined = if stderr.is_empty() { stdout } else { stderr };
            if is_not_a_git_repo_stderr(&combined) {
                return Err(GitError::NotAGitRepository(self.repo_root.clone()));
            }
            let lower = combined.to_ascii_lowercase();
            if lower.contains("nothing to commit")
                || lower.contains("no changes added to commit")
                || lower.contains("nothing added to commit")
            {
                return Err(GitError::NothingToCommit);
            }
            return Err(GitError::CommandFailed {
                command: "git commit -m <msg>".to_string(),
                code: out.status.code(),
                stderr: combined,
            });
        }

        let commits = self.log(1)?;
        if let Some(latest) = commits.into_iter().next() {
            Ok(latest)
        } else {
            Ok(CommitInfo {
                hash: "HEAD".to_string(),
                author: String::new(),
                message: trimmed.to_string(),
                timestamp: 0,
            })
        }
    }

    fn log(&self, limit: usize) -> Result<Vec<CommitInfo>, GitError> {
        if limit == 0 {
            return Ok(Vec::new());
        }
        let limit_str = limit.to_string();
        let out = self.run_git(&[
            "log",
            "-n",
            &limit_str,
            "--format=%H%x1f%an <%ae>%x1f%at%x1f%s%x1e",
        ])?;
        if !out.status.success() {
            let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
            if is_not_a_git_repo_stderr(&stderr) {
                return Err(GitError::NotAGitRepository(self.repo_root.clone()));
            }
            let lower = stderr.to_ascii_lowercase();
            if lower.contains("does not have any commits yet")
                || lower.contains("bad default revision 'head'")
                || lower.contains("unknown revision")
            {
                return Ok(Vec::new());
            }
            return Err(GitError::CommandFailed {
                command: "git log".to_string(),
                code: out.status.code(),
                stderr,
            });
        }

        let raw = String::from_utf8_lossy(&out.stdout);
        Ok(parse_git_log_records(&raw))
    }
}

fn is_not_a_git_repo_stderr(stderr: &str) -> bool {
    let lower = stderr.to_ascii_lowercase();
    lower.contains("not a git repository") || lower.contains("cannot change to")
}

fn is_missing_head_object_stderr(stderr: &str) -> bool {
    let lower = stderr.to_ascii_lowercase();
    lower.contains("does not exist in 'head'")
        || lower.contains("exists on disk, but not in 'head'")
        || lower.contains("invalid object name 'head'")
        || lower.contains("bad revision 'head'")
        || lower.contains("ambiguous argument 'head'")
        || lower.contains("path '")
}

fn parse_git_log_records(raw: &str) -> Vec<CommitInfo> {
    let mut commits = Vec::new();
    for record in raw.split('\x1e') {
        let trimmed = record.trim();
        if trimmed.is_empty() {
            continue;
        }
        let parts: Vec<&str> = trimmed.split('\x1f').collect();
        if parts.len() >= 4 {
            let hash = parts[0].trim().to_string();
            let author = parts[1].trim().to_string();
            let timestamp = parts[2].trim().parse::<u64>().unwrap_or(0);
            let message = parts[3].trim().to_string();
            commits.push(CommitInfo {
                hash,
                author,
                message,
                timestamp,
            });
        }
    }
    commits
}

fn status_kind_from_porcelain_char(ch: char, is_unmerged: bool) -> Option<GitStatusKind> {
    if is_unmerged {
        return Some(GitStatusKind::Conflicted);
    }
    match ch {
        '.' => None,
        'M' | 'T' => Some(GitStatusKind::Modified),
        'A' | 'C' => Some(GitStatusKind::Added),
        'D' => Some(GitStatusKind::Deleted),
        'R' => Some(GitStatusKind::Renamed),
        'U' => Some(GitStatusKind::Conflicted),
        '?' => Some(GitStatusKind::Untracked),
        _ => Some(GitStatusKind::Modified),
    }
}

/// Parses `git status --porcelain=v2 -b -z` NUL-delimited byte output.
pub fn parse_porcelain_v2_z(bytes: &[u8]) -> Result<GitStatusSnapshot, GitError> {
    let records: Vec<String> = bytes
        .split(|&b| b == 0)
        .filter(|s| !s.is_empty())
        .map(|s| String::from_utf8_lossy(s).into_owned())
        .collect();
    parse_porcelain_v2_records(&records, true)
}

/// Parses newline-delimited `git status --porcelain=v2 -b` text output (supporting C-quoted paths).
pub fn parse_porcelain_v2(text: &str) -> Result<GitStatusSnapshot, GitError> {
    let records: Vec<String> = text
        .lines()
        .map(|l| l.trim_end_matches('\r').to_string())
        .filter(|l| !l.is_empty())
        .collect();
    parse_porcelain_v2_records(&records, false)
}

fn parse_porcelain_v2_records(
    records: &[String],
    nul_delimited: bool,
) -> Result<GitStatusSnapshot, GitError> {
    let mut branch = BranchInfo {
        name: String::new(),
        is_current: true,
        upstream: None,
    };
    let mut head_oid: Option<String> = None;
    let mut ahead: u32 = 0;
    let mut behind: u32 = 0;
    let mut staged = Vec::new();
    let mut changes = Vec::new();
    let mut untracked = Vec::new();

    let mut i = 0;
    while i < records.len() {
        let rec = &records[i];
        i += 1;

        if let Some(rest) = rec.strip_prefix("# ") {
            if let Some(oid) = rest.strip_prefix("branch.oid ") {
                let oid = oid.trim();
                head_oid = if oid == "(initial)" || oid.is_empty() {
                    None
                } else {
                    Some(oid.to_string())
                };
            } else if let Some(head) = rest.strip_prefix("branch.head ") {
                branch.name = head.trim().to_string();
            } else if let Some(upstream) = rest.strip_prefix("branch.upstream ") {
                let u = upstream.trim();
                branch.upstream = if u.is_empty() {
                    None
                } else {
                    Some(u.to_string())
                };
            } else if let Some(ab) = rest.strip_prefix("branch.ab ") {
                for tok in ab.split_whitespace() {
                    if let Some(a) = tok.strip_prefix('+') {
                        ahead = a.parse::<u32>().unwrap_or(0);
                    } else if let Some(b) = tok.strip_prefix('-') {
                        behind = b.parse::<u32>().unwrap_or(0);
                    }
                }
            }
            continue;
        }

        if let Some(raw_path) = rec.strip_prefix("? ") {
            let path = if nul_delimited {
                raw_path.to_string()
            } else {
                unquote_git_path(raw_path)
            };
            untracked.push(GitStatusEntry {
                path: PathBuf::from(path),
                status: GitStatusKind::Untracked,
                is_staged: false,
            });
            continue;
        }

        if rec.starts_with("! ") {
            continue;
        }

        if rec.starts_with("1 ") {
            let parts: Vec<&str> = rec.splitn(9, ' ').collect();
            if parts.len() < 9 {
                return Err(GitError::ParseError(format!(
                    "malformed porcelain v2 type-1 entry: {}",
                    rec
                )));
            }
            let xy = parts[1];
            let raw_path = parts[8];
            let path_str = if nul_delimited {
                raw_path.to_string()
            } else {
                unquote_git_path(raw_path)
            };
            let path = PathBuf::from(path_str);

            let mut xy_chars = xy.chars();
            let x = xy_chars.next().unwrap_or('.');
            let y = xy_chars.next().unwrap_or('.');

            if let Some(kind) = status_kind_from_porcelain_char(x, false) {
                staged.push(GitStatusEntry {
                    path: path.clone(),
                    status: kind,
                    is_staged: true,
                });
            }
            if let Some(kind) = status_kind_from_porcelain_char(y, false) {
                changes.push(GitStatusEntry {
                    path,
                    status: kind,
                    is_staged: false,
                });
            }
            continue;
        }

        if rec.starts_with("2 ") {
            let parts: Vec<&str> = rec.splitn(10, ' ').collect();
            if parts.len() < 10 {
                return Err(GitError::ParseError(format!(
                    "malformed porcelain v2 type-2 entry: {}",
                    rec
                )));
            }
            let xy = parts[1];
            let raw_path_field = parts[9];
            let path_str = if nul_delimited {
                if i < records.len() {
                    i += 1;
                }
                raw_path_field.to_string()
            } else {
                let new_part = raw_path_field
                    .split('\t')
                    .next()
                    .unwrap_or(raw_path_field);
                unquote_git_path(new_part)
            };
            let path = PathBuf::from(path_str);

            let mut xy_chars = xy.chars();
            let x = xy_chars.next().unwrap_or('.');
            let y = xy_chars.next().unwrap_or('.');

            if let Some(kind) = status_kind_from_porcelain_char(x, false) {
                staged.push(GitStatusEntry {
                    path: path.clone(),
                    status: kind,
                    is_staged: true,
                });
            }
            if let Some(kind) = status_kind_from_porcelain_char(y, false) {
                changes.push(GitStatusEntry {
                    path,
                    status: kind,
                    is_staged: false,
                });
            }
            continue;
        }

        if rec.starts_with("u ") {
            let parts: Vec<&str> = rec.splitn(11, ' ').collect();
            if parts.len() < 11 {
                return Err(GitError::ParseError(format!(
                    "malformed porcelain v2 unmerged entry: {}",
                    rec
                )));
            }
            let raw_path = parts[10];
            let path_str = if nul_delimited {
                raw_path.to_string()
            } else {
                unquote_git_path(raw_path)
            };
            changes.push(GitStatusEntry {
                path: PathBuf::from(path_str),
                status: GitStatusKind::Conflicted,
                is_staged: false,
            });
        }
    }

    Ok(GitStatusSnapshot {
        branch,
        head_oid,
        ahead,
        behind,
        staged,
        changes,
        untracked,
    })
}

fn unquote_git_path(raw: &str) -> String {
    if !(raw.starts_with('"') && raw.ends_with('"') && raw.len() >= 2) {
        return raw.to_string();
    }
    let inner = &raw[1..raw.len() - 1];
    let mut bytes = Vec::with_capacity(inner.len());
    let mut chars = inner.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\\' {
            let mut buf = [0u8; 4];
            bytes.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
            continue;
        }
        match chars.next() {
            Some('n') => bytes.push(b'\n'),
            Some('r') => bytes.push(b'\r'),
            Some('t') => bytes.push(b'\t'),
            Some('b') => bytes.push(0x08),
            Some('f') => bytes.push(0x0c),
            Some('a') => bytes.push(0x07),
            Some('v') => bytes.push(0x0b),
            Some('\\') => bytes.push(b'\\'),
            Some('"') => bytes.push(b'"'),
            Some(d1 @ '0'..='7') => {
                let mut val = (d1 as u8) - b'0';
                for _ in 0..2 {
                    if let Some(&d @ '0'..='7') = chars.peek() {
                        chars.next();
                        val = val * 8 + ((d as u8) - b'0');
                    } else {
                        break;
                    }
                }
                bytes.push(val);
            }
            Some(other) => {
                let mut buf = [0u8; 4];
                bytes.extend_from_slice(other.encode_utf8(&mut buf).as_bytes());
            }
            None => bytes.push(b'\\'),
        }
    }
    String::from_utf8_lossy(&bytes).into_owned()
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

    #[test]
    fn test_parse_porcelain_v2_z_branches_renames_spaces_unmerged() {
        let mut raw = Vec::new();
        for rec in [
            "# branch.oid (initial)",
            "# branch.head main",
            "# branch.upstream origin/main",
            "# branch.ab +3 -2",
            "1 .M N... 100644 100644 100644 a1 b2 src/file with spaces.rs",
            "1 MM N... 100644 100644 100644 a1 b2 src/both.rs",
            "2 R. N... 100644 100644 100644 a1 b2 R100 new dir/renamed file.rs",
            "old dir/original file.rs",
            "u UU N... 100644 100644 100644 100644 a1 b2 c3 conflict.txt",
            "? untracked with space.md",
            "! target/ignored.o",
        ] {
            raw.extend_from_slice(rec.as_bytes());
            raw.push(0);
        }

        let snap = parse_porcelain_v2_z(&raw).unwrap();
        assert_eq!(snap.branch.name, "main");
        assert_eq!(snap.branch.upstream.as_deref(), Some("origin/main"));
        assert_eq!(snap.head_oid, None);
        assert_eq!(snap.ahead, 3);
        assert_eq!(snap.behind, 2);

        assert_eq!(snap.staged.len(), 2);
        assert_eq!(snap.staged[0].path, PathBuf::from("src/both.rs"));
        assert_eq!(snap.staged[0].status, GitStatusKind::Modified);
        assert_eq!(
            snap.staged[1].path,
            PathBuf::from("new dir/renamed file.rs")
        );
        assert_eq!(snap.staged[1].status, GitStatusKind::Renamed);

        assert_eq!(snap.changes.len(), 3);
        assert_eq!(
            snap.changes[0].path,
            PathBuf::from("src/file with spaces.rs")
        );
        assert_eq!(snap.changes[0].status, GitStatusKind::Modified);
        assert_eq!(snap.changes[1].path, PathBuf::from("src/both.rs"));
        assert_eq!(snap.changes[2].path, PathBuf::from("conflict.txt"));
        assert_eq!(snap.changes[2].status, GitStatusKind::Conflicted);

        assert_eq!(snap.untracked.len(), 1);
        assert_eq!(
            snap.untracked[0].path,
            PathBuf::from("untracked with space.md")
        );
    }

    #[test]
    fn test_parse_porcelain_v2_quoted_paths() {
        let text = "\
# branch.oid 0123456789abcdef0123456789abcdef01234567\n\
# branch.head feature/test\n\
1 M. N... 100644 100644 100644 111 222 \"dir/file with \\\"quotes\\\".rs\"\n\
2 R. N... 100644 100644 100644 111 222 R100 \"new name.txt\"\t\"old name.txt\"\n";

        let snap = parse_porcelain_v2(text).unwrap();
        assert_eq!(snap.branch.name, "feature/test");
        assert_eq!(
            snap.head_oid.as_deref(),
            Some("0123456789abcdef0123456789abcdef01234567")
        );
        assert_eq!(snap.staged.len(), 2);
        assert_eq!(
            snap.staged[0].path,
            PathBuf::from("dir/file with \"quotes\".rs")
        );
        assert_eq!(snap.staged[1].path, PathBuf::from("new name.txt"));
        assert_eq!(snap.staged[1].status, GitStatusKind::Renamed);
    }

    #[test]
    fn test_in_memory_git_backend_trait() {
        let backend = InMemoryGitBackend::new(ScmRepository::sample_repository());
        let dyn_backend: &dyn GitBackend = &backend;
        assert!(dyn_backend.is_git_repo());
        assert_eq!(dyn_backend.current_branch().unwrap().name, "main");
        assert_eq!(
            dyn_backend.show_head(Path::new("Cargo.toml")).unwrap().as_deref(),
            Some("[package]\nname = \"demo\"\nversion = \"0.1.0\"\n")
        );
        dyn_backend.stage_all().unwrap();
        let c = dyn_backend.commit("test commit").unwrap();
        assert_eq!(c.message, "test commit");
        assert_eq!(dyn_backend.log(5).unwrap().len(), 1);
    }

    struct TempDirGuard(PathBuf);
    impl TempDirGuard {
        fn new(prefix: &str) -> Self {
            let unique = format!(
                "{}_{}_{}",
                prefix,
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_nanos()
            );
            let dir = std::env::temp_dir().join(unique);
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }
    }
    impl Drop for TempDirGuard {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn run_git_helper(dir: &Path, args: &[&str]) {
        let mut cmd = Command::new("git");
        cmd.arg("-C").arg(dir).args(args);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x0800_0000);
        }
        let out = cmd.output().expect("git binary must be available for test");
        assert!(
            out.status.success(),
            "git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&out.stderr)
        );
    }

    #[test]
    fn test_git_cli_backend_real_temp_repo_lifecycle() {
        let temp = TempDirGuard::new("rustcode_git_cli_test");
        let dir = &temp.0;

        // 1. Non-git directory handling
        let backend = GitCliBackend::new(dir);
        assert!(!backend.is_git_repo());
        assert!(matches!(
            backend.status(),
            Err(GitError::NotAGitRepository(_))
        ));

        // 2. Missing git binary handling
        let missing_bin = GitCliBackend::with_git_binary(dir, "definitely-missing-git-binary-xyz");
        assert!(!missing_bin.is_git_repo());
        assert!(matches!(missing_bin.status(), Err(GitError::GitNotFound)));

        // 3. Init empty repo (unborn HEAD)
        run_git_helper(dir, &["init", "-b", "main"]);
        run_git_helper(dir, &["config", "user.name", "RustCode Test"]);
        run_git_helper(dir, &["config", "user.email", "test@rustcode.local"]);
        run_git_helper(dir, &["config", "commit.gpgsign", "false"]);

        assert!(backend.is_git_repo());
        let snap0 = backend.status().unwrap();
        assert_eq!(snap0.branch.name, "main");
        assert!(snap0.head_oid.is_none());
        assert!(snap0.staged.is_empty());
        assert!(snap0.changes.is_empty());
        assert!(snap0.untracked.is_empty());
        assert!(backend.log(10).unwrap().is_empty());
        assert_eq!(backend.show_head(Path::new("hello world.txt")).unwrap(), None);

        // 4. Untracked file with spaces -> stage -> unstage on unborn HEAD -> stage -> commit
        let file_rel = Path::new("hello world.txt");
        let file_abs = dir.join(file_rel);
        std::fs::write(&file_abs, "line 1\n").unwrap();

        let snap1 = backend.status().unwrap();
        assert_eq!(snap1.untracked.len(), 1);
        assert_eq!(snap1.untracked[0].path, PathBuf::from("hello world.txt"));

        backend.stage(file_rel).unwrap();
        let snap2 = backend.status().unwrap();
        assert_eq!(snap2.staged.len(), 1);
        assert_eq!(snap2.staged[0].status, GitStatusKind::Added);
        assert!(snap2.untracked.is_empty());

        // Unstage on unborn HEAD must work cleanly
        backend.unstage(file_rel).unwrap();
        let snap3 = backend.status().unwrap();
        assert!(snap3.staged.is_empty());
        assert_eq!(snap3.untracked.len(), 1);

        backend.stage_all().unwrap();
        let commit1 = backend.commit("initial commit").unwrap();
        assert_eq!(commit1.message, "initial commit");
        assert!(!commit1.hash.is_empty());

        // 5. Modify file -> diff & show HEAD -> stage -> unstage -> commit
        std::fs::write(&file_abs, "line 1\nline 2\n").unwrap();
        let snap4 = backend.status().unwrap();
        assert_eq!(snap4.changes.len(), 1);
        assert_eq!(snap4.changes[0].status, GitStatusKind::Modified);

        let unstaged_diff = backend.diff(file_rel, false).unwrap();
        assert!(unstaged_diff.contains("+line 2"));
        assert_eq!(
            backend.show_head(file_rel).unwrap().as_deref(),
            Some("line 1\n")
        );

        backend.stage(file_rel).unwrap();
        let staged_diff = backend.diff(file_rel, true).unwrap();
        assert!(staged_diff.contains("+line 2"));

        backend.unstage(file_rel).unwrap();
        assert_eq!(backend.status().unwrap().changes.len(), 1);

        backend.stage(file_rel).unwrap();
        let commit2 = backend.commit("second commit").unwrap();
        assert_eq!(commit2.message, "second commit");
        assert_eq!(backend.log(10).unwrap().len(), 2);

        // 6. Rename + Delete + ScmRepository::from_git_cli
        run_git_helper(dir, &["mv", "hello world.txt", "renamed file.txt"]);
        let snap_rename = backend.status().unwrap();
        assert_eq!(snap_rename.staged.len(), 1);
        assert_eq!(snap_rename.staged[0].status, GitStatusKind::Renamed);
        assert_eq!(
            snap_rename.staged[0].path,
            PathBuf::from("renamed file.txt")
        );
        backend.commit("rename file").unwrap();

        std::fs::remove_file(dir.join("renamed file.txt")).unwrap();
        let repo_model = ScmRepository::from_git_cli(dir).unwrap();
        assert_eq!(repo_model.current_branch.name, "main");
        assert_eq!(repo_model.changes.len(), 1);
        assert_eq!(repo_model.changes[0].status, GitStatusKind::Deleted);
        assert_eq!(repo_model.commits.len(), 3);

        // 7. Detached HEAD handling
        run_git_helper(dir, &["checkout", "--detach", "HEAD"]);
        let snap_detached = backend.status().unwrap();
        assert_eq!(snap_detached.branch.name, "(detached)");
    }
}
