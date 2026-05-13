const DESTRUCTIVE_PATTERNS: &[&str] = &[
    "rm ",
    "rm -",
    "mv ",
    "cp ",
    "chmod ",
    "chown ",
    "dd ",
    "mkfs",
    "fdisk",
    "parted",
    "kill ",
    "killall ",
    "pkill ",
    "shutdown",
    "reboot",
    ">:",
    "> ",
    ">> ",
    "truncate ",
    "git reset",
    "git clean",
    "git checkout ",
    "docker rm",
    "docker rmi",
    "kubectl delete",
];

pub fn looks_destructive(command: &str) -> bool {
    let normalized = normalize(command);
    DESTRUCTIVE_PATTERNS
        .iter()
        .any(|pattern| normalized.contains(pattern))
}

fn normalize(command: &str) -> String {
    let mut out = String::with_capacity(command.len() + 2);
    out.push(' ');
    for ch in command.chars() {
        if ch.is_whitespace() {
            out.push(' ');
        } else {
            out.push(ch.to_ascii_lowercase());
        }
    }
    out.push(' ');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flags_destructive_commands() {
        assert!(looks_destructive("rm -rf target"));
        assert!(looks_destructive("git clean -fdx"));
        assert!(looks_destructive("kubectl delete pod foo"));
    }

    #[test]
    fn allows_read_only_commands() {
        assert!(!looks_destructive(
            "git ls-tree -r -l --full-name HEAD | sort -n -k 4 -r | head -n 10"
        ));
        assert!(!looks_destructive("find . -type f -size +10M -print"));
    }
}
