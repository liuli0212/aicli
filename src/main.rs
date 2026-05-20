use aicli::config::{config_template, AppConfig};
use aicli::environment::{missing_command_heads, EnvironmentSnapshot};
use aicli::executor;
use aicli::llm::{create_generator, GeneratedCommand};
use aicli::prompt::RequestContext;
use aicli::safety::looks_destructive;
use clap::Parser;
use dialoguer::{Confirm, Input};
use std::fs;
use std::io::IsTerminal;
use std::path::PathBuf;
use std::process::{Command as ProcessCommand, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Parser)]
#[command(
    name = "aicli",
    version,
    about = "Generate and run shell commands from natural language."
)]
struct Cli {
    /// Natural-language description of the command you want.
    prompt: Vec<String>,

    /// Provider name from config, or one of: openai_compat, gemini.
    #[arg(short, long)]
    provider: Option<String>,

    /// Override the provider model.
    #[arg(short, long)]
    model: Option<String>,

    /// Config file path. Defaults to ./config.toml, ~/.config/aicli/config.toml, then ~/.aicli/config.toml.
    #[arg(short, long)]
    config: Option<PathBuf>,

    /// Generate and print the command without prompting or executing.
    #[arg(long)]
    dry_run: bool,

    /// Execute the generated command without interactive editing.
    #[arg(short = 'y', long)]
    yes: bool,

    /// Shell used to run the command. Defaults to $SHELL, then /bin/sh.
    #[arg(long)]
    shell: Option<String>,

    /// Print a deployable config.toml template and exit.
    #[arg(long)]
    config_template: bool,

    /// Print request diagnostics to stderr.
    #[arg(short, long)]
    verbose: bool,
}

#[tokio::main]
async fn main() {
    if let Err(error) = run().await {
        eprintln!("error: {error}");
        std::process::exit(1);
    }
}

async fn run() -> Result<(), AppError> {
    let cli = Cli::parse();
    if cli.config_template {
        print!("{}", config_template());
        return Ok(());
    }

    if cli.prompt.is_empty() {
        return Err(AppError::MissingPrompt);
    }

    let description = cli.prompt.join(" ");
    let verbose = cli.verbose;
    let (config, config_source) = AppConfig::load_with_source(cli.config.as_deref())?;
    let provider = cli
        .provider
        .or_else(|| config.default_provider.clone())
        .unwrap_or_else(|| "gemini".to_string());
    let shell = cli
        .shell
        .or_else(|| std::env::var("SHELL").ok())
        .unwrap_or_else(|| "/bin/sh".to_string());

    let environment = EnvironmentSnapshot::collect();
    let context = RequestContext {
        cwd: std::env::current_dir()?.display().to_string(),
        shell: shell.clone(),
        os: std::env::consts::OS.to_string(),
        available_commands: environment.available_commands.clone(),
        missing_commands: environment.missing_commands.clone(),
        capability_notes: environment.capability_notes.clone(),
        limitations: environment.limitations.clone(),
        git_root: environment.git_root.clone(),
        git_branch: environment.git_branch.clone(),
    };

    log_verbose(
        verbose,
        format!(
            "config={}",
            config_source
                .as_ref()
                .map(|path| path.display().to_string())
                .unwrap_or_else(|| "none (using built-in defaults)".to_string())
        ),
    );
    log_verbose(verbose, format!("provider={provider}"));
    log_verbose(verbose, format!("cwd={}", context.cwd));
    log_verbose(verbose, format!("shell={}", context.shell));
    log_verbose(
        verbose,
        format!(
            "available_commands={}",
            context.available_commands.join(",")
        ),
    );
    log_verbose(
        verbose,
        format!("missing_commands={}", context.missing_commands.join(",")),
    );
    log_verbose(
        verbose,
        format!("capability_notes={}", context.capability_notes.join(";")),
    );
    log_verbose(
        verbose,
        format!("limitations={}", context.limitations.join(";")),
    );
    if let Some(git_root) = &context.git_root {
        log_verbose(verbose, format!("git_root={git_root}"));
    }
    if let Some(git_branch) = &context.git_branch {
        log_verbose(verbose, format!("git_branch={git_branch}"));
    }
    log_proxy_env(verbose);

    let generator = create_generator(&provider, cli.model, &config, verbose)?;
    let generated = generator.generate(&description, &context).await?;

    if cli.dry_run {
        print_generated(&generated);
        print_missing_command_warning(&generated.command);
        return Ok(());
    }

    let command = if cli.yes {
        print_generated(&generated);
        generated.command.clone()
    } else {
        prompt_for_command(&generated)?
    };

    if command.trim().is_empty() {
        println!("Canceled.");
        return Ok(());
    }

    warn_missing_commands(&command);

    if looks_destructive(&command)
        && !cli.yes
        && !Confirm::new()
            .with_prompt("This command may change or remove data. Execute it?")
            .default(false)
            .interact()?
    {
        println!("Canceled.");
        return Ok(());
    }

    print_output_header();
    let status = executor::execute(&shell, &command)?;
    if !status.success() {
        let code = status
            .code()
            .map(|code| code.to_string())
            .unwrap_or_else(|| "signal".to_string());
        return Err(AppError::CommandFailed(code));
    }

    Ok(())
}

fn print_generated(generated: &GeneratedCommand) {
    print_explanation(generated);
    print_command_block(&generated.command);
}

fn prompt_for_command(generated: &GeneratedCommand) -> Result<String, AppError> {
    print_explanation(generated);
    print_command_block(&generated.command);

    if is_multiline(&generated.command) {
        return prompt_for_script(&generated.command);
    }

    let command = Input::<String>::new()
        .with_prompt("Edit command, then press Enter")
        .with_initial_text(generated.command.clone())
        .allow_empty(true)
        .interact_text()?;

    Ok(command.trim().to_string())
}

fn prompt_for_script(script: &str) -> Result<String, AppError> {
    let action = Input::<String>::new()
        .with_prompt("Press Enter to run, type 'e' to edit, or 'q' to cancel")
        .allow_empty(true)
        .interact_text()?;

    match action.trim() {
        "" => Ok(script.trim().to_string()),
        "e" | "E" => edit_script(script),
        "q" | "Q" => Ok(String::new()),
        other => Ok(other.to_string()),
    }
}

fn edit_script(script: &str) -> Result<String, AppError> {
    let editor = std::env::var("VISUAL")
        .or_else(|_| std::env::var("EDITOR"))
        .unwrap_or_else(|_| "vi".to_string());
    let path = temp_script_path();

    fs::write(&path, script)?;
    let editor_command = format!("{} {}", editor, shell_quote_path(&path));
    let status = ProcessCommand::new("/bin/sh")
        .arg("-lc")
        .arg(editor_command)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()?;

    if !status.success() {
        let _ = fs::remove_file(&path);
        return Err(AppError::EditorFailed(
            status
                .code()
                .map(|code| code.to_string())
                .unwrap_or_else(|| "signal".to_string()),
        ));
    }

    let edited = fs::read_to_string(&path)?;
    let _ = fs::remove_file(&path);
    Ok(edited.trim().to_string())
}

fn print_explanation(generated: &GeneratedCommand) {
    if generated.explanation.is_empty() {
        return;
    }

    println!("{}", style_bold("Explanation"));
    println!("  {}", generated.explanation);
    println!();
}

fn print_command_block(command: &str) {
    let label = if is_multiline(command) {
        "Script"
    } else {
        "Command"
    };
    println!("{}", style_bold(label));
    println!("{}", style_dim("----------------------------------------"));
    if is_multiline(command) {
        for line in command.lines() {
            println!("{}", style_command(line));
        }
    } else {
        println!("{} {}", style_dim("$"), style_command(command));
    }
    println!("{}", style_dim("----------------------------------------"));
    println!();
}

fn print_output_header() {
    println!("{}", style_bold("Output"));
    println!("{}", style_dim("----------------------------------------"));
}

fn warn_missing_commands(command: &str) {
    let missing = missing_command_heads(command);
    if missing.is_empty() {
        return;
    }

    print_missing_command_warning_with_list(&missing);
}

fn print_missing_command_warning(command: &str) {
    let missing = missing_command_heads(command);
    if !missing.is_empty() {
        print_missing_command_warning_with_list(&missing);
    }
}

fn print_missing_command_warning_with_list(missing: &[String]) {
    println!("{}", style_bold("Warning"));
    println!(
        "  This machine does not appear to have: {}",
        missing.join(", ")
    );
    println!("  This is only a PATH-based hint; aliases, shell functions, or other environments may still work.");
    println!();
}

fn log_verbose(verbose: bool, message: impl AsRef<str>) {
    if verbose {
        eprintln!("[aicli] {}", message.as_ref());
    }
}

fn log_proxy_env(verbose: bool) {
    if !verbose {
        return;
    }

    for name in [
        "https_proxy",
        "HTTPS_PROXY",
        "http_proxy",
        "HTTP_PROXY",
        "all_proxy",
        "ALL_PROXY",
        "no_proxy",
        "NO_PROXY",
    ] {
        match std::env::var(name) {
            Ok(value) if !value.trim().is_empty() => {
                eprintln!("[aicli] env {name}={}", redact_url_userinfo(&value));
            }
            _ => eprintln!("[aicli] env {name}=<unset>"),
        }
    }
}

fn redact_url_userinfo(input: &str) -> String {
    let mut output = input.to_string();
    let mut search_start = 0;

    while let Some(relative_scheme) = output[search_start..].find("://") {
        let authority_start = search_start + relative_scheme + 3;
        let authority_end = output[authority_start..]
            .find(|ch: char| ch == '/' || ch == '?' || ch == '#' || ch.is_whitespace())
            .map(|offset| authority_start + offset)
            .unwrap_or_else(|| output.len());

        if let Some(relative_at) = output[authority_start..authority_end].find('@') {
            let at = authority_start + relative_at;
            output.replace_range(authority_start..at, "***");
            search_start = authority_start + 4;
        } else {
            search_start = authority_end;
        }
    }

    output
}

fn style_bold(text: &str) -> String {
    style(text, "1")
}

fn style_dim(text: &str) -> String {
    style(text, "2")
}

fn style_command(text: &str) -> String {
    style(text, "1;36")
}

fn style(text: &str, code: &str) -> String {
    if use_color() {
        format!("\x1b[{code}m{text}\x1b[0m")
    } else {
        text.to_string()
    }
}

fn use_color() -> bool {
    std::io::stdout().is_terminal() && std::env::var_os("NO_COLOR").is_none()
}

fn is_multiline(command: &str) -> bool {
    command.lines().count() > 1
}

fn temp_script_path() -> PathBuf {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or(0);
    std::env::temp_dir().join(format!(
        "aicli-script-{}-{timestamp}.sh",
        std::process::id()
    ))
}

fn shell_quote_path(path: &std::path::Path) -> String {
    let raw = path.to_string_lossy();
    format!("'{}'", raw.replace('\'', "'\\''"))
}

#[derive(Debug, thiserror::Error)]
enum AppError {
    #[error(transparent)]
    Config(#[from] aicli::config::ConfigError),
    #[error(transparent)]
    Llm(#[from] aicli::llm::LlmError),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Dialoguer(#[from] dialoguer::Error),
    #[error("command exited with status {0}")]
    CommandFailed(String),
    #[error("editor exited with status {0}")]
    EditorFailed(String),
    #[error("missing prompt. Try: aicli \"看看当前git项目哪些文件很大\"")]
    MissingPrompt,
}
