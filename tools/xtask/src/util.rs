//! Shared helpers for xtask subcommands (owned by WP-01): error type, repository paths,
//! process execution and a small reader for Cargo dependency tables.

use std::fmt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// Result of an xtask subcommand.
pub type Result<T = ()> = std::result::Result<T, Error>;

/// Why an xtask subcommand failed. Every variant maps to exit code 1.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// Wrong arguments; the message includes the expected usage.
    Usage(String),
    /// Any other failure, with a message that says how to fix it.
    Failed(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Usage(msg) => write!(f, "usage: {msg}"),
            Error::Failed(msg) => f.write_str(msg),
        }
    }
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Failed(format!("I/O error: {e}"))
    }
}

/// Builds `Error::Failed` from anything printable.
pub fn fail(msg: impl Into<String>) -> Error {
    Error::Failed(msg.into())
}

/// Repository root: the parent of `tools/` (xtask lives in `tools/xtask`).
pub fn repo_root() -> PathBuf {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    manifest_dir
        .ancestors()
        .nth(2)
        .map(Path::to_path_buf)
        .unwrap_or_else(|| manifest_dir.to_path_buf())
}

/// `cargo` from the environment cargo exported to us (respects rust-toolchain.toml).
pub fn cargo() -> Command {
    let mut cmd = Command::new(std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into()));
    cmd.current_dir(repo_root());
    cmd
}

/// Renders a command line for logs.
pub fn display_command(cmd: &Command) -> String {
    let mut parts = vec![cmd.get_program().to_string_lossy().into_owned()];
    parts.extend(cmd.get_args().map(|a| a.to_string_lossy().into_owned()));
    parts.join(" ")
}

/// Runs a command with inherited stdio; fails with `what` when it exits non-zero.
pub fn run(cmd: &mut Command, what: &str) -> Result {
    eprintln!("xtask: $ {}", display_command(cmd));
    let status = cmd.status().map_err(|e| {
        fail(format!(
            "{what}: could not start {}: {e}",
            display_command(cmd)
        ))
    })?;
    if status.success() {
        Ok(())
    } else {
        Err(fail(format!("{what} failed ({status})")))
    }
}

/// Captured output of a finished command.
pub struct Captured {
    pub success: bool,
    pub stdout: String,
    pub stderr: String,
}

/// Runs a command and captures stdout and stderr.
pub fn capture(cmd: &mut Command) -> Result<Captured> {
    let out = cmd
        .stdin(Stdio::null())
        .output()
        .map_err(|e| fail(format!("could not start {}: {e}", display_command(cmd))))?;
    Ok(Captured {
        success: out.status.success(),
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
    })
}

/// Splits `args` into flags (`--x`) and positionals, rejecting flags not in `allowed`.
pub fn parse_flags<'a>(
    command: &str,
    args: &'a [String],
    allowed: &[&str],
) -> Result<(Vec<&'a str>, Vec<&'a str>)> {
    let mut flags = Vec::new();
    let mut positional = Vec::new();
    for arg in args {
        if arg.starts_with("--") {
            if !allowed.contains(&arg.as_str()) {
                return Err(Error::Usage(format!(
                    "cargo xtask {command} [{}] (unknown flag {arg})",
                    allowed.join("] [")
                )));
            }
            flags.push(arg.as_str());
        } else {
            positional.push(arg.as_str());
        }
    }
    Ok((flags, positional))
}

/// One entry of a Cargo dependency table, as far as the pin check needs it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DepSpec {
    pub name: String,
    /// `version` requirement (`"=1.2.3"`), `None` for path/workspace entries.
    pub version: Option<String>,
    /// Explicit `default-features`, `None` when not written.
    pub default_features: Option<bool>,
    /// The entry declares a `path`.
    pub path: bool,
}

/// Removes a trailing `# comment` that is not inside a string.
fn strip_comment(line: &str) -> &str {
    let mut in_str = false;
    for (i, c) in line.char_indices() {
        match c {
            '"' => in_str = !in_str,
            '#' if !in_str => return &line[..i],
            _ => {}
        }
    }
    line
}

/// Splits `a = 1, b = { c = 2 }, d = [1, 2]` at top-level commas.
fn split_top_level(s: &str) -> Vec<&str> {
    let (mut depth, mut in_str, mut start) = (0i32, false, 0usize);
    let mut parts = Vec::new();
    for (i, c) in s.char_indices() {
        match c {
            '"' => in_str = !in_str,
            '{' | '[' if !in_str => depth += 1,
            '}' | ']' if !in_str => depth -= 1,
            ',' if !in_str && depth == 0 => {
                parts.push(s[start..i].trim());
                start = i + 1;
            }
            _ => {}
        }
    }
    let last = s[start..].trim();
    if !last.is_empty() {
        parts.push(last);
    }
    parts
}

fn unquote(v: &str) -> Option<String> {
    let v = v.trim();
    v.strip_prefix('"')
        .and_then(|r| r.strip_suffix('"'))
        .map(str::to_owned)
}

/// Reads the dependency table `[table]` (e.g. `dependencies`, `workspace.dependencies`)
/// of a Cargo manifest. Supports the single-line forms used in this repository:
/// `name = "=x.y.z"`, `name = { version = "=x.y.z", default-features = false, ... }` and
/// `name.workspace = true`. A multi-line inline table is reported as an error rather
/// than misread.
pub fn read_dep_table(manifest: &str, table: &str) -> Result<Vec<DepSpec>> {
    let header = format!("[{table}]");
    let mut inside = false;
    let mut deps = Vec::new();
    for (lineno, raw) in manifest.lines().enumerate() {
        let line = strip_comment(raw).trim();
        if line.is_empty() {
            continue;
        }
        if line.starts_with('[') {
            inside = line == header;
            continue;
        }
        if !inside {
            continue;
        }
        let (key, value) = line.split_once('=').ok_or_else(|| {
            fail(format!(
                "line {}: expected `name = ...` in [{table}]",
                lineno + 1
            ))
        })?;
        let key = key.trim();
        let value = value.trim();
        if let Some(name) = key.strip_suffix(".workspace") {
            deps.push(DepSpec {
                name: name.trim().to_owned(),
                version: None,
                default_features: None,
                path: false,
            });
            continue;
        }
        let mut spec = DepSpec {
            name: key.to_owned(),
            version: None,
            default_features: None,
            path: false,
        };
        if let Some(v) = unquote(value) {
            spec.version = Some(v);
        } else if let Some(inner) = value.strip_prefix('{').and_then(|r| r.strip_suffix('}')) {
            for part in split_top_level(inner) {
                let Some((k, v)) = part.split_once('=') else {
                    continue;
                };
                match k.trim() {
                    "version" => spec.version = unquote(v),
                    "default-features" | "default_features" => {
                        spec.default_features = Some(v.trim() == "true")
                    }
                    "path" => spec.path = true,
                    _ => {}
                }
            }
        } else {
            return Err(fail(format!(
                "line {}: unsupported dependency syntax for `{key}` in [{table}] \
                 (use a single-line string or inline table)",
                lineno + 1
            )));
        }
        deps.push(spec);
    }
    Ok(deps)
}

#[cfg(test)]
mod tests {
    use super::*;

    const MANIFEST: &str = r##"
[package]
name = "x" # not a dep
[dependencies]
# comment line
serde = { version = "=1.0.229", features = ["derive"] }   # trailing "#" comment
snow = { version = "=0.10.0", default-features = false }
hkdf = "=0.12.4"
mesh-wire = { path = "../mesh-wire" }
nnnoiseless.workspace = true
[dev-dependencies]
proptest = "=1.12.0"
"##;

    #[test]
    fn reads_the_requested_table_only() {
        let deps = read_dep_table(MANIFEST, "dependencies").expect("parses");
        let names: Vec<_> = deps.iter().map(|d| d.name.as_str()).collect();
        assert_eq!(names, ["serde", "snow", "hkdf", "mesh-wire", "nnnoiseless"]);
        assert_eq!(deps[0].version.as_deref(), Some("=1.0.229"));
        assert_eq!(deps[0].default_features, None);
        assert_eq!(deps[1].default_features, Some(false));
        assert_eq!(deps[2].version.as_deref(), Some("=0.12.4"));
        assert!(deps[3].path && deps[3].version.is_none());
        assert!(!deps[4].path && deps[4].version.is_none());
        let dev = read_dep_table(MANIFEST, "dev-dependencies").expect("parses");
        assert_eq!(dev.len(), 1);
        assert_eq!(dev[0].version.as_deref(), Some("=1.12.0"));
    }

    #[test]
    fn multi_line_inline_tables_are_rejected() {
        let bad = "[dependencies]\nfoo = { version = \"=1.0.0\",\n";
        assert!(read_dep_table(bad, "dependencies").is_err());
    }

    #[test]
    fn split_respects_nesting_and_strings() {
        assert_eq!(
            split_top_level(r#"a = "x,y", b = [1, 2], c = { d = 1, e = 2 }"#),
            [r#"a = "x,y""#, "b = [1, 2]", "c = { d = 1, e = 2 }"]
        );
    }
}
