//! The agent's system prompt.

use std::fmt::Write as _;

use warp_multi_agent_api as api;

use crate::render::truncate_middle;
use crate::tools::ToolKind;

/// Project rule files (AGENTS.md, WARP.md, ...) can be long; they share this budget.
const MAX_RULES_CHARS: usize = 12_000;

const BASE: &str = "\
You are the AI agent of LocalShell, a terminal. You help the user with software engineering \
and command-line tasks on their own machine.

# How to work
- Find facts with tools instead of guessing: read files, search, and run read-only commands.
- Before editing a file, read the part you are changing. Make focused, minimal edits.
- Prefer the file tools over shell redirection (echo/sed/cat >) for creating and editing files.
- Shell commands run in the user's live terminal session. Avoid interactive programs and \
pagers. Start servers and watchers with wait_until_complete=false, then poll their output.
- Classify every command's risk honestly; the user relies on it to decide what runs without \
confirmation.
- If a command or edit fails, read the error and adjust instead of repeating it.
- Ask the user only when a decision is genuinely theirs to make.
- Be concise. Use Markdown, and put commands and code in fenced blocks. When you finish a \
task, briefly say what you did and anything left for the user.";

const PLAN_MODE: &str = "\
# Plan mode
The user wants a plan, not changes. Investigate with read-only tools, then reply with a \
numbered, concrete plan. Do not edit files or run commands that change anything.";

pub(crate) fn system_prompt(
    context: Option<&api::InputContext>,
    tools: &[ToolKind],
    plan_mode: bool,
) -> String {
    let mut prompt = BASE.to_owned();
    if tools.contains(&ToolKind::UpdateTodos) {
        prompt.push_str(
            "\n- For tasks with three or more steps, keep a todo list with update_todos and \
             mark items completed as you go.",
        );
    }
    if plan_mode {
        prompt.push_str("\n\n");
        prompt.push_str(PLAN_MODE);
    }
    if let Some(context) = context {
        let environment = environment(context);
        if !environment.is_empty() {
            prompt.push_str("\n\n# Environment\n");
            prompt.push_str(&environment);
        }
        let rules = project_rules(context);
        if !rules.is_empty() {
            prompt.push_str(
                "\n\n# Project rules\nThe user's project defines these instructions. Follow \
                 them.\n\n",
            );
            prompt.push_str(&rules);
        }
    }
    prompt
}

fn environment(context: &api::InputContext) -> String {
    let mut text = String::new();
    if let Some(os) = &context.operating_system {
        let name = [os.platform.as_str(), os.distribution.as_str()]
            .into_iter()
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>()
            .join(" ");
        if !name.is_empty() {
            let _ = writeln!(text, "- Operating system: {name}");
        }
    }
    if let Some(shell) = &context.shell
        && !shell.name.is_empty()
    {
        let _ = writeln!(text, "- Shell: {} {}", shell.name, shell.version);
    }
    if let Some(directory) = &context.directory {
        if !directory.pwd.is_empty() {
            let _ = writeln!(text, "- Current directory: {}", directory.pwd);
        }
        if !directory.home.is_empty() {
            let _ = writeln!(text, "- Home directory: {}", directory.home);
        }
    }
    if let Some(git) = &context.git {
        let mut line = String::new();
        if !git.branch.is_empty() {
            let _ = write!(line, "branch {}", git.branch);
        }
        if let Some(repository) = &git.repository
            && !repository.name.is_empty()
        {
            let _ = write!(line, " in {}/{}", repository.owner, repository.name);
        }
        if !line.is_empty() {
            let _ = writeln!(text, "- Git: {}", line.trim());
        }
    }
    if let Some(time) = &context.current_time
        && let Some(time) = chrono::DateTime::from_timestamp(time.seconds, time.nanos.max(0) as u32)
    {
        let _ = writeln!(
            text,
            "- Current time (UTC): {}",
            time.format("%Y-%m-%d %H:%M")
        );
    }
    text.trim_end().to_owned()
}

fn project_rules(context: &api::InputContext) -> String {
    let mut sections = Vec::new();
    for rules in &context.project_rules {
        for file in &rules.active_rule_files {
            if !file.content.trim().is_empty() {
                sections.push(format!("## {}\n{}", file.file_path, file.content.trim()));
            }
        }
    }
    truncate_middle(&sections.join("\n\n"), MAX_RULES_CHARS)
}
