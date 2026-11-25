//! Command-line interface parser for the `rustcode` binary.

use serde::{Deserialize, Serialize};
use std::ffi::OsString;
use std::fmt;
use std::path::PathBuf;

/// Target file and 1-based cursor position parsed from `--goto <file:line[:col]>`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GotoTarget {
    pub path: PathBuf,
    pub line: usize,
    pub column: usize,
}

impl GotoTarget {
    pub fn line_zero_based(&self) -> usize {
        self.line.saturating_sub(1)
    }

    pub fn col_zero_based(&self) -> usize {
        self.column.saturating_sub(1)
    }
}

/// Resolved startup execution mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartupMode {
    Headless,
    Desktop,
}

/// Parsed CLI flags and positional paths.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CliOptions {
    pub paths: Vec<PathBuf>,
    pub goto: Vec<GotoTarget>,
    pub headless: bool,
    pub gui: bool,
    pub new_window: bool,
    pub reuse_window: bool,
    pub user_data_dir: Option<PathBuf>,
    pub extensions_dir: Option<PathBuf>,
    pub disable_extensions: bool,
}

impl CliOptions {
    /// Determine whether to launch the headless bootstrap or desktop `winit` event loop.
    ///
    /// Preserves the F17 `scenario_09` & `M2-01` invariant:
    /// - Zero positional paths / `--goto` / `--gui`, OR `--headless` -> `StartupMode::Headless`.
    /// - When `desktop` feature is compiled AND `--headless` is NOT set AND (`--gui` or
    ///   positional `[path...]` or `--goto`) -> `StartupMode::Desktop`.
    pub fn startup_mode(&self, desktop_compiled: bool) -> StartupMode {
        if self.headless {
            StartupMode::Headless
        } else if desktop_compiled
            && (self.gui || !self.paths.is_empty() || !self.goto.is_empty())
        {
            StartupMode::Desktop
        } else {
            StartupMode::Headless
        }
    }
}

/// Top-level CLI outcome.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CliAction {
    ShowHelp,
    ShowVersion,
    Run(CliOptions),
}

/// Structured CLI parse error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CliError {
    UnknownFlag(String),
    MissingValue(String),
    InvalidGoto(String),
    EmptyValue(String),
}

impl fmt::Display for CliError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CliError::UnknownFlag(flag) => write!(f, "Unknown option: {flag}"),
            CliError::MissingValue(flag) => write!(f, "Missing value for option: {flag}"),
            CliError::InvalidGoto(spec) => {
                write!(f, "Invalid --goto location '{spec}' (expected <file>[:line[:col]])")
            }
            CliError::EmptyValue(flag) => write!(f, "Empty value for option: {flag}"),
        }
    }
}

impl std::error::Error for CliError {}

pub fn version_text() -> String {
    format!("rustcode {}", env!("CARGO_PKG_VERSION"))
}

pub fn usage_text() -> &'static str {
    "\
Usage: rustcode [OPTIONS] [PATH...]

Options:
  -h, --help                  Print help information
  -V, --version               Print version information
      --headless              Run headless bootstrap and exit
      --gui                   Launch native desktop window (requires desktop feature)
  -n, --new-window            Force opening a new desktop window
  -r, --reuse-window          Force reusing the last active desktop window
  -g, --goto <FILE:LINE[:COL]>
                              Open file at 1-based line and optional column
      --user-data-dir <DIR>   Override user data/config directory
      --extensions-dir <DIR>  Override user extensions directory
      --disable-extensions    Disable loading and running extensions"
}

/// Parse arguments from `std::env::args_os()` (skipping argv[0]).
pub fn parse_env_args() -> Result<CliAction, CliError> {
    parse_cli_args(std::env::args_os().skip(1))
}

/// Parse an iterator of CLI arguments (without argv[0]).
pub fn parse_cli_args<I, S>(args: I) -> Result<CliAction, CliError>
where
    I: IntoIterator<Item = S>,
    S: Into<OsString>,
{
    let mut opts = CliOptions::default();
    let mut iter = args.into_iter().map(Into::into);
    let mut positional_only = false;

    while let Some(raw) = iter.next() {
        let Some(s) = raw.to_str() else {
            // Non-UTF-8 paths are valid positional filesystem paths.
            opts.paths.push(PathBuf::from(raw));
            continue;
        };

        if positional_only {
            opts.paths.push(PathBuf::from(raw));
            continue;
        }

        if s == "--" {
            positional_only = true;
            continue;
        }

        match s {
            "-h" | "--help" => return Ok(CliAction::ShowHelp),
            "-V" | "--version" => return Ok(CliAction::ShowVersion),
            "--headless" => opts.headless = true,
            "--gui" => opts.gui = true,
            "-n" | "--new-window" => opts.new_window = true,
            "-r" | "--reuse-window" => opts.reuse_window = true,
            "--disable-extensions" => opts.disable_extensions = true,
            "-g" | "--goto" => {
                let val = iter
                    .next()
                    .ok_or_else(|| CliError::MissingValue(s.to_string()))?;
                let val_str = val
                    .to_str()
                    .ok_or_else(|| CliError::InvalidGoto(val.to_string_lossy().into_owned()))?;
                opts.goto.push(parse_goto_spec(val_str)?);
            }
            "--user-data-dir" => {
                let val = iter
                    .next()
                    .ok_or_else(|| CliError::MissingValue(s.to_string()))?;
                if val.is_empty() {
                    return Err(CliError::EmptyValue(s.to_string()));
                }
                opts.user_data_dir = Some(PathBuf::from(val));
            }
            "--extensions-dir" => {
                let val = iter
                    .next()
                    .ok_or_else(|| CliError::MissingValue(s.to_string()))?;
                if val.is_empty() {
                    return Err(CliError::EmptyValue(s.to_string()));
                }
                opts.extensions_dir = Some(PathBuf::from(val));
            }
            _ if s.starts_with("--goto=") => {
                let val = &s["--goto=".len()..];
                opts.goto.push(parse_goto_spec(val)?);
            }
            _ if s.starts_with("--user-data-dir=") => {
                let val = &s["--user-data-dir=".len()..];
                if val.is_empty() {
                    return Err(CliError::EmptyValue("--user-data-dir".to_string()));
                }
                opts.user_data_dir = Some(PathBuf::from(val));
            }
            _ if s.starts_with("--extensions-dir=") => {
                let val = &s["--extensions-dir=".len()..];
                if val.is_empty() {
                    return Err(CliError::EmptyValue("--extensions-dir".to_string()));
                }
                opts.extensions_dir = Some(PathBuf::from(val));
            }
            _ if s.starts_with('-') && s.len() > 1 => {
                return Err(CliError::UnknownFlag(s.to_string()));
            }
            _ => {
                opts.paths.push(PathBuf::from(raw));
            }
        }
    }

    Ok(CliAction::Run(opts))
}

fn has_windows_drive_prefix(s: &str) -> bool {
    let bytes = s.as_bytes();
    if bytes.len() < 2 || !bytes[0].is_ascii_alphabetic() || bytes[1] != b':' {
        return false;
    }
    if bytes.len() == 2 {
        return true;
    }
    if bytes[2] == b'\\' || bytes[2] == b'/' {
        return true;
    }
    // Distinguish drive-relative `C:foo.rs:10:2` from Unix `a:15` / `a:15:2` where the
    // segment immediately after `[A-Za-z]:` is purely digits.
    let first_segment = s[2..].split(':').next().unwrap_or("");
    !first_segment.is_empty() && !first_segment.chars().all(|c| c.is_ascii_digit())
}

/// Parse `<file>[:line[:col]]`, handling Windows drive-letter colons cleanly.
pub fn parse_goto_spec(spec: &str) -> Result<GotoTarget, CliError> {
    if spec.trim().is_empty() {
        return Err(CliError::InvalidGoto(spec.to_string()));
    }

    let (drive_prefix, rest) = if has_windows_drive_prefix(spec) {
        (&spec[..2], &spec[2..])
    } else {
        ("", spec)
    };

    if rest.is_empty() && !drive_prefix.is_empty() {
        return Err(CliError::InvalidGoto(spec.to_string()));
    }

    let make_path = |tail: &str| -> Result<PathBuf, CliError> {
        let mut full = String::with_capacity(drive_prefix.len() + tail.len());
        full.push_str(drive_prefix);
        full.push_str(tail);
        if full.is_empty() {
            return Err(CliError::InvalidGoto(spec.to_string()));
        }
        Ok(PathBuf::from(full))
    };

    let Some((before_last, last)) = rest.rsplit_once(':') else {
        return Ok(GotoTarget {
            path: make_path(rest)?,
            line: 1,
            column: 1,
        });
    };

    if last.is_empty() || !last.chars().all(|c| c.is_ascii_digit()) {
        return Err(CliError::InvalidGoto(spec.to_string()));
    }
    let last_num: usize = last
        .parse()
        .map_err(|_| CliError::InvalidGoto(spec.to_string()))?;

    if let Some((file_part, maybe_line)) = before_last.rsplit_once(':') {
        if !maybe_line.is_empty() && maybe_line.chars().all(|c| c.is_ascii_digit()) {
            let line_num: usize = maybe_line
                .parse()
                .map_err(|_| CliError::InvalidGoto(spec.to_string()))?;
            return Ok(GotoTarget {
                path: make_path(file_part)?,
                line: line_num.max(1),
                column: last_num.max(1),
            });
        }
    }

    Ok(GotoTarget {
        path: make_path(before_last)?,
        line: last_num.max(1),
        column: 1,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cli_zero_arg_and_headless_startup_mode() {
        let empty = match parse_cli_args(Vec::<&str>::new()).unwrap() {
            CliAction::Run(opts) => opts,
            other => panic!("expected Run, got {other:?}"),
        };
        assert!(!empty.headless);
        assert_eq!(empty.startup_mode(false), StartupMode::Headless);
        // Zero-arg must stay Headless even when compiled with --features desktop (F17 scenario_09 / M2-01)
        assert_eq!(empty.startup_mode(true), StartupMode::Headless);

        let headless_with_path = match parse_cli_args(["--headless", "src/main.rs"]).unwrap() {
            CliAction::Run(opts) => opts,
            other => panic!("expected Run, got {other:?}"),
        };
        assert!(headless_with_path.headless);
        assert_eq!(headless_with_path.startup_mode(true), StartupMode::Headless);

        let gui = match parse_cli_args(["--gui"]).unwrap() {
            CliAction::Run(opts) => opts,
            other => panic!("expected Run, got {other:?}"),
        };
        assert_eq!(gui.startup_mode(false), StartupMode::Headless);
        assert_eq!(gui.startup_mode(true), StartupMode::Desktop);

        let path_only = match parse_cli_args(["workspace_dir"]).unwrap() {
            CliAction::Run(opts) => opts,
            other => panic!("expected Run, got {other:?}"),
        };
        assert_eq!(path_only.startup_mode(false), StartupMode::Headless);
        assert_eq!(path_only.startup_mode(true), StartupMode::Desktop);
    }

    #[test]
    fn test_cli_version_help_and_overrides() {
        assert_eq!(parse_cli_args(["--version"]).unwrap(), CliAction::ShowVersion);
        assert_eq!(parse_cli_args(["-V"]).unwrap(), CliAction::ShowVersion);
        assert_eq!(parse_cli_args(["--help"]).unwrap(), CliAction::ShowHelp);
        assert_eq!(parse_cli_args(["-h"]).unwrap(), CliAction::ShowHelp);
        assert!(version_text().starts_with("rustcode "));

        let opts = match parse_cli_args([
            "--user-data-dir",
            "/tmp/udata",
            "--extensions-dir=/tmp/exts",
            "--disable-extensions",
            "dir_a",
            "file_b.rs",
        ])
        .unwrap()
        {
            CliAction::Run(o) => o,
            other => panic!("expected Run, got {other:?}"),
        };
        assert_eq!(opts.user_data_dir, Some(PathBuf::from("/tmp/udata")));
        assert_eq!(opts.extensions_dir, Some(PathBuf::from("/tmp/exts")));
        assert!(opts.disable_extensions);
        assert_eq!(
            opts.paths,
            vec![PathBuf::from("dir_a"), PathBuf::from("file_b.rs")]
        );
    }

    #[test]
    fn test_cli_goto_parsing_including_windows_drive_letters() {
        let g1 = parse_goto_spec("foo.rs:42").unwrap();
        assert_eq!(g1.path, PathBuf::from("foo.rs"));
        assert_eq!(g1.line, 42);
        assert_eq!(g1.column, 1);
        assert_eq!(g1.line_zero_based(), 41);
        assert_eq!(g1.col_zero_based(), 0);

        let g2 = parse_goto_spec("foo.rs:42:10").unwrap();
        assert_eq!(g2.path, PathBuf::from("foo.rs"));
        assert_eq!(g2.line_zero_based(), 41);
        assert_eq!(g2.col_zero_based(), 9);

        let win_full = parse_goto_spec(r"C:\dir\file.rs:15:3").unwrap();
        assert_eq!(win_full.path, PathBuf::from(r"C:\dir\file.rs"));
        assert_eq!(win_full.line, 15);
        assert_eq!(win_full.column, 3);
        assert_eq!(win_full.line_zero_based(), 14);
        assert_eq!(win_full.col_zero_based(), 2);

        let win_no_pos = parse_goto_spec(r"D:\workspace\lib.rs").unwrap();
        assert_eq!(win_no_pos.path, PathBuf::from(r"D:\workspace\lib.rs"));
        assert_eq!(win_no_pos.line, 1);
        assert_eq!(win_no_pos.column, 1);

        let win_fwd = parse_goto_spec("C:/dir/file.rs:9").unwrap();
        assert_eq!(win_fwd.path, PathBuf::from("C:/dir/file.rs"));
        assert_eq!(win_fwd.line, 9);
        assert_eq!(win_fwd.column, 1);

        let short_unix = parse_goto_spec("a:15").unwrap();
        assert_eq!(short_unix.path, PathBuf::from("a"));
        assert_eq!(short_unix.line, 15);
        assert_eq!(short_unix.column, 1);

        let short_unix_col = parse_goto_spec("a:15:2").unwrap();
        assert_eq!(short_unix_col.path, PathBuf::from("a"));
        assert_eq!(short_unix_col.line, 15);
        assert_eq!(short_unix_col.column, 2);

        let drive_rel = parse_goto_spec("C:foo.rs:10:2").unwrap();
        assert_eq!(drive_rel.path, PathBuf::from("C:foo.rs"));
        assert_eq!(drive_rel.line, 10);
        assert_eq!(drive_rel.column, 2);
    }

    #[test]
    fn test_cli_errors_on_unknown_flags_and_missing_values() {
        assert_eq!(
            parse_cli_args(["--bogus-flag"]).unwrap_err(),
            CliError::UnknownFlag("--bogus-flag".to_string())
        );
        assert_eq!(
            parse_cli_args(["--goto"]).unwrap_err(),
            CliError::MissingValue("--goto".to_string())
        );
        assert_eq!(
            parse_cli_args(["--user-data-dir"]).unwrap_err(),
            CliError::MissingValue("--user-data-dir".to_string())
        );
        assert_eq!(
            parse_cli_args(["--extensions-dir"]).unwrap_err(),
            CliError::MissingValue("--extensions-dir".to_string())
        );
        assert!(matches!(
            parse_cli_args(["--goto", "foo.rs:notnum"]).unwrap_err(),
            CliError::InvalidGoto(_)
        ));
        assert!(matches!(
            parse_cli_args(["--goto", ""]).unwrap_err(),
            CliError::InvalidGoto(_)
        ));
    }

    #[test]
    fn test_cli_new_window_and_reuse_window_preserve_headless_zero_arg_contract() {
        let zero = match parse_cli_args::<[&str; 0], &str>([]).unwrap() {
            CliAction::Run(o) => o,
            other => panic!("unexpected {other:?}"),
        };
        assert_eq!(zero.startup_mode(true), StartupMode::Headless);
        assert!(!zero.new_window);
        assert!(!zero.reuse_window);

        let headless_new = match parse_cli_args(["--headless", "--new-window", "foo.rs"]).unwrap() {
            CliAction::Run(o) => o,
            other => panic!("unexpected {other:?}"),
        };
        assert!(headless_new.new_window);
        assert_eq!(headless_new.startup_mode(true), StartupMode::Headless);

        let reuse = match parse_cli_args(["-r", "bar.rs"]).unwrap() {
            CliAction::Run(o) => o,
            other => panic!("unexpected {other:?}"),
        };
        assert!(reuse.reuse_window);
        assert!(!reuse.new_window);
        assert_eq!(reuse.startup_mode(true), StartupMode::Desktop);
    }
}
