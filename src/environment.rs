use std::collections::BTreeSet;
use std::fs;
use std::path::Path;
use std::process::{Command, Stdio};

const COMMAND_CANDIDATES: &[&str] = &[
    "awk", "bat", "cat", "cargo", "curl", "cut", "docker", "du", "dust", "eza", "fd", "fdfind",
    "find", "git", "grep", "head", "jq", "less", "ls", "lsof", "ncdu", "netstat", "node", "npm",
    "perl", "ps", "python", "python3", "rg", "ripgrep", "rsync", "rustc", "sed", "sort", "ss",
    "stat", "tail", "tar", "tee", "tree", "tr", "unzip", "wc", "wget", "xargs", "yq", "zip",
];

const SHELL_BUILTINS_AND_KEYWORDS: &[&str] = &[
    "!", ".", ":", "[", "alias", "bg", "break", "case", "cd", "command", "continue", "declare",
    "do", "done", "echo", "else", "env", "eval", "exec", "exit", "export", "false", "fg", "fi",
    "for", "function", "getopts", "hash", "help", "history", "if", "in", "jobs", "let", "local",
    "popd", "printf", "pushd", "pwd", "read", "readonly", "return", "select", "set", "shift",
    "source", "test", "then", "time", "trap", "true", "type", "ulimit", "umask", "unalias",
    "unset", "until", "wait", "while",
];

#[derive(Debug, Clone)]
pub struct EnvironmentSnapshot {
    pub available_commands: Vec<String>,
    pub missing_commands: Vec<String>,
    pub capability_notes: Vec<String>,
    pub limitations: Vec<String>,
    pub git_root: Option<String>,
    pub git_branch: Option<String>,
}

impl EnvironmentSnapshot {
    pub fn collect() -> Self {
        let mut available_commands = Vec::new();
        let mut missing_commands = Vec::new();

        for command in COMMAND_CANDIDATES {
            if command_exists(command) {
                available_commands.push((*command).to_string());
            } else {
                missing_commands.push((*command).to_string());
            }
        }

        let git_root = git_output(["rev-parse", "--show-toplevel"]);
        let git_branch = git_output(["branch", "--show-current"]);
        let capability_probe = CapabilityProbe::collect();

        Self {
            available_commands,
            missing_commands,
            capability_notes: capability_probe.notes,
            limitations: capability_probe.limitations,
            git_root,
            git_branch,
        }
    }
}

#[derive(Debug, Clone)]
struct CapabilityProbe {
    notes: Vec<String>,
    limitations: Vec<String>,
}

impl CapabilityProbe {
    fn collect() -> Self {
        let find_printf = command_succeeds("find", &[".", "-maxdepth", "0", "-printf", "%p\n"]);
        let sort_h = command_succeeds("sort", &["-h"]);
        let stat_gnu = command_succeeds("stat", &["-c", "%s", "."]);
        let stat_bsd = command_succeeds("stat", &["-f", "%z", "."]);
        let readlink_f = command_succeeds("readlink", &["-f", "."]);
        let du_h = command_succeeds("du", &["-h", "."]);
        let du_k = command_succeeds("du", &["-k", "."]);
        let realpath = command_exists("realpath");

        let stat_style = match (stat_gnu, stat_bsd) {
            (true, _) => "gnu",
            (false, true) => "bsd",
            _ => "unknown",
        };

        let mut notes = vec![
            format!("find_printf={}", yes_no(find_printf)),
            format!("sort_h={}", yes_no(sort_h)),
            format!("stat_style={stat_style}"),
            format!("readlink_f={}", yes_no(readlink_f)),
            format!("realpath={}", yes_no(realpath)),
            format!("du_h={}", yes_no(du_h)),
            format!("du_k={}", yes_no(du_k)),
        ];

        if let Some(uname) = command_output("uname", &["-s"]) {
            notes.push(format!("uname={uname}"));
        }

        let mut limitations = Vec::new();
        if !find_printf {
            limitations.push(
                "find -printf is unavailable; avoid GNU find -printf and use find -exec du/stat instead"
                    .to_string(),
            );
        }
        if !sort_h {
            limitations.push(
                "sort -h is unavailable; sort numeric byte/KB values with sort -n or sort -nr"
                    .to_string(),
            );
        }
        if stat_style == "bsd" {
            limitations.push("use BSD stat syntax like stat -f %z, not GNU stat -c".to_string());
        } else if stat_style == "gnu" {
            limitations.push("use GNU stat syntax like stat -c %s, not BSD stat -f".to_string());
        }
        if !readlink_f {
            limitations.push(
                "readlink -f is unavailable; avoid it unless realpath is available".to_string(),
            );
        }

        Self { notes, limitations }
    }
}

pub fn missing_command_heads(command: &str) -> Vec<String> {
    let tokens = tokenize_shell_heads(command);
    let mut missing = BTreeSet::new();
    let mut expecting_command = true;

    for token in tokens {
        match token.as_str() {
            "|" | "||" | "&&" | ";" | "(" | ")" => {
                expecting_command = true;
                continue;
            }
            _ => {}
        }

        if !expecting_command {
            continue;
        }

        if is_assignment(&token) {
            continue;
        }

        expecting_command = false;

        if is_known_shell_word(&token) {
            continue;
        }

        if !command_exists(&token) {
            missing.insert(token);
        }
    }

    missing.into_iter().collect()
}

pub fn command_exists(command: &str) -> bool {
    if command.contains('/') {
        return is_executable(Path::new(command));
    }

    let Some(paths) = std::env::var_os("PATH") else {
        return false;
    };

    std::env::split_paths(&paths).any(|dir| is_executable(&dir.join(command)))
}

fn git_output<const N: usize>(args: [&str; N]) -> Option<String> {
    if !command_exists("git") {
        return None;
    }

    let output = Command::new("git")
        .args(args)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (!text.is_empty()).then_some(text)
}

fn command_succeeds(command: &str, args: &[&str]) -> bool {
    if !command_exists(command) {
        return false;
    }

    Command::new(command)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

fn command_output(command: &str, args: &[&str]) -> Option<String> {
    if !command_exists(command) {
        return None;
    }

    let output = Command::new(command)
        .args(args)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (!text.is_empty()).then_some(text)
}

fn yes_no(value: bool) -> &'static str {
    if value {
        "yes"
    } else {
        "no"
    }
}

fn tokenize_shell_heads(command: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut chars = command.chars().peekable();
    let mut quote: Option<char> = None;
    let mut escaped = false;

    while let Some(ch) = chars.next() {
        if escaped {
            current.push(ch);
            escaped = false;
            continue;
        }

        if ch == '\\' {
            escaped = true;
            continue;
        }

        if let Some(active_quote) = quote {
            if ch == active_quote {
                quote = None;
            } else {
                current.push(ch);
            }
            continue;
        }

        if ch == '\'' || ch == '"' {
            quote = Some(ch);
            continue;
        }

        if ch == '\n' {
            push_token(&mut tokens, &mut current);
            tokens.push(";".to_string());
            continue;
        }

        if ch.is_whitespace() {
            push_token(&mut tokens, &mut current);
            continue;
        }

        match ch {
            '|' | '&' => {
                push_token(&mut tokens, &mut current);
                if chars.peek() == Some(&ch) {
                    chars.next();
                    tokens.push(format!("{ch}{ch}"));
                } else {
                    tokens.push(ch.to_string());
                }
            }
            ';' | '(' | ')' => {
                push_token(&mut tokens, &mut current);
                tokens.push(ch.to_string());
            }
            _ => current.push(ch),
        }
    }

    push_token(&mut tokens, &mut current);
    tokens
}

fn push_token(tokens: &mut Vec<String>, current: &mut String) {
    if current.is_empty() {
        return;
    }

    tokens.push(std::mem::take(current));
}

fn is_assignment(token: &str) -> bool {
    let Some((name, value)) = token.split_once('=') else {
        return false;
    };

    !name.is_empty()
        && !value.is_empty()
        && name
            .chars()
            .all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
        && name
            .chars()
            .next()
            .is_some_and(|ch| ch == '_' || ch.is_ascii_alphabetic())
}

fn is_known_shell_word(token: &str) -> bool {
    SHELL_BUILTINS_AND_KEYWORDS.contains(&token)
}

fn is_executable(path: &Path) -> bool {
    if !path.is_file() {
        return false;
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::metadata(path)
            .map(|metadata| metadata.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
    }

    #[cfg(not(unix))]
    {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_missing_command_heads() {
        let missing = missing_command_heads("definitely-not-aicli-command --version | head -n 1");

        assert_eq!(missing, vec!["definitely-not-aicli-command"]);
    }

    #[test]
    fn ignores_assignments_and_shell_builtins() {
        let missing = missing_command_heads("FOO=bar echo hello && cd /tmp");

        assert!(missing.is_empty());
    }

    #[test]
    fn keeps_pipeline_command_heads() {
        let missing = missing_command_heads("find . -type f | definitely-not-aicli-sorter");

        assert_eq!(missing, vec!["definitely-not-aicli-sorter"]);
    }

    #[test]
    fn treats_newline_as_command_separator() {
        let missing = missing_command_heads("echo ok\ndefinitely-not-aicli-command");

        assert_eq!(missing, vec!["definitely-not-aicli-command"]);
    }
}
