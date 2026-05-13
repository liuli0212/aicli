use std::process::{Command, ExitStatus, Stdio};

pub fn execute(shell: &str, command: &str) -> std::io::Result<ExitStatus> {
    Command::new(shell)
        .arg("-lc")
        .arg(command)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
}
