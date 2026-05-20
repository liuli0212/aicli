#[derive(Debug, Clone)]
pub struct RequestContext {
    pub cwd: String,
    pub shell: String,
    pub os: String,
    pub available_commands: Vec<String>,
    pub missing_commands: Vec<String>,
    pub capability_notes: Vec<String>,
    pub limitations: Vec<String>,
    pub git_root: Option<String>,
    pub git_branch: Option<String>,
}

pub fn system_prompt() -> &'static str {
    r#"You turn a user's natural-language request into exactly one shell command.

Return only JSON in this shape:
{"command":"...","explanation":"..."}

Rules:
- The command must be directly executable by a POSIX-like shell.
- Prefer read-only inspection commands when the request asks to look, list, search, count, or explain.
- Do not wrap the command in markdown.
- Do not include comments, placeholders, or multiple alternatives.
- Do not use sudo unless the user explicitly asks for privileged system changes.
- Avoid destructive operations unless the user explicitly asks for them.
- If the request is ambiguous, choose the safest useful command.
- Use the current working directory unless the user specifies another path.
- Prefer commands listed as available in the execution context.
- Avoid commands listed as missing in the execution context.
- Follow the capability notes and limitations from the execution context; do not assume GNU/Linux behavior on macOS/BSD.
- Prefer portable, widely supported command options.
- Avoid GNU-only flags such as `find -printf`, `sort -h`, `stat -c`, or `readlink -f` unless the capability notes say they are supported.
- For requests like "top 10 large files in the current directory", prefer a portable pipeline like:
  find . -type f -exec du -k {} + | sort -nr | head -n 10
- For requests like "which files are large in the current Git project", use a pipeline like:
  git ls-tree -r -l --full-name HEAD | sort -n -k 4 -r | head -n 10
- Do not use `git ls-tree --sort`, because `git ls-tree` does not support sorting on common Git versions.
"#
}

pub fn user_prompt(description: &str, context: &RequestContext) -> String {
    let git_context = match (&context.git_root, &context.git_branch) {
        (Some(root), Some(branch)) => format!("- git_root: {root}\n- git_branch: {branch}\n"),
        (Some(root), None) => format!("- git_root: {root}\n"),
        _ => "- git_root: not inside a git repository or git unavailable\n".to_string(),
    };

    format!(
        "User request:\n{description}\n\nExecution context:\n- cwd: {cwd}\n- shell: {shell}\n- os: {os}\n{git_context}- available_commands: {available_commands}\n- missing_commands_to_avoid: {missing_commands}\n- capability_notes: {capability_notes}\n- limitations: {limitations}\n\nGenerate one command.",
        description = description.trim(),
        cwd = context.cwd,
        shell = context.shell,
        os = context.os,
        git_context = git_context,
        available_commands = context.available_commands.join(", "),
        missing_commands = context.missing_commands.join(", "),
        capability_notes = context.capability_notes.join("; "),
        limitations = context.limitations.join("; "),
    )
}
