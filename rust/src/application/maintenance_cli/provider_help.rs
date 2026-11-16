//! `--help` text for `many-ai-cli provider ...`. The `add` text is written for an
//! AI agent that was asked to register a new AI CLI: it says what to research,
//! what to show the user, what the user (not the agent) answers, and how to
//! check the result.

pub const OVERVIEW: &str = "\
many-ai-cli provider - register, list, remove and test AI CLIs

USAGE
  many-ai-cli provider add --id <id> --command <executable> [options]   register a new AI CLI
  many-ai-cli provider list                                             list every AI CLI
  many-ai-cli provider remove <id>                                      remove one you registered
  many-ai-cli provider test <id> [--prompt <text>] [--timeout <sec>]    run its headless mode once
  many-ai-cli provider backup list|verify|restore <id> ...              revision backups
  many-ai-cli provider reset --distributed <id> --expected-revision <r> built-in back to shipped
  many-ai-cli provider recover <id> [revision|--list]                   recover a broken definition

If a Hub is running (inside a Hub session, or started by this user), add / remove / restore / reset /
recover go through it: it validates, keeps a backup, reloads, and tells every open dashboard. With no
Hub they edit the provider files directly and the Hub reads them on its next start. Nothing here
prints the Hub token.

Adding a CLI as an AI agent: read `many-ai-cli provider add --help` first.
";

pub const ADD: &str = "\
many-ai-cli provider add - register another AI CLI

USAGE
  many-ai-cli provider add --id <id> --command <executable> [options]

OPTIONS
  --id <id>                 lowercase letters, digits, '.', '-' and '_'; starts with a letter or digit;
                            at most 64 characters. Not a built-in id (claude, codex,
                            copilot, cursor-agent, opencode, grok, command-code) and not \"shell\".
  --command <executable>    what to run. Must be on PATH (or an absolute path). Not a shell line.
  --label <name>            name shown in the dashboard (default: the id)
  --arg <a>                 one argument of the interactive launch; repeat for several
  --headless-arg <a>        one argument of the non-interactive (headless) launch; repeat
  --headless-format <f>     text (default) or claude-stream-json
  --prompt-via <arg|stdin>  how headless mode receives the prompt (default arg: appended after
                            the headless arguments)
  --description <text>      one line shown in Settings
  --approval-option <text>  an option string the CLI prints on its approval prompts, exactly as
                            printed (e.g. \"Allow once\"); repeat. Written to a phrase file under
                            the many-ai-cli home and read by the Hub for this CLI.
  Giving any --headless-arg, --headless-format or --prompt-via makes the CLI usable as an
  `orchestrate spawn --execution-mode headless` child.

  A value that starts with '-' is fine: --arg --interactive, --headless-arg -p.

IF YOU ARE AN AI AGENT ASKED TO ADD A NEW CLI
  Do not guess. Find these in the CLI's own --help and official documentation, and show the user
  what you found before you register it. For every item say where it came from (the official
  documentation page, the CLI's --help, or \"not found, so not set\").
  1. The command that starts the interactive UI (this becomes --command and --arg).
  2. Its non-interactive flag (the --headless-arg values) and whether the prompt is passed as an
     argument or on stdin (--prompt-via).
  3. Its model flag, if any. A fixed model can be put in --arg; the dashboard does not pass a
     per-session model to a registered CLI.
  4. The exact option strings its approval prompts print. Copy them verbatim into
     --approval-option; without them only the generic wording (Yes/No/Allow/Deny) is detected.
  5. Where it keeps its home or config. If it needs a different HOME or a config directory,
     write a small wrapper script that sets it and registers the script as --command.
  6. Whether the CLI's terms of service allow another tool to drive it. Report what the terms
     say and where; the user decides. If the user says no, do not register it. Do not decide
     this for the user.
  Things you must not do: print, read aloud or ask for the Hub token (the command reads it
  itself); answer a first-run \"do you trust this folder\" or a login prompt of the new CLI
  (the user answers those in the dashboard session); keep retrying with different flags if
  `provider test` fails - show the user the output and ask.

  No approval from the user is awaited when you run this command. With a Hub running, the
  dashboard shows \"<name> was added (command: ...)\" with an Undo, so the user keeps control
  without an extra step. Tell the user what you added and the command line that will run.

AFTER ADDING
  Run `many-ai-cli provider test <id>`: it runs the headless mode once with a small prompt and
  prints the exit code, the seconds taken and the first lines of output. `provider list` shows
  the result; `provider remove <id>` undoes it.

OUTPUT
  On success: the id, the exact executable and arguments that will run (interactive and
  headless), whether headless is available, and whether a Hub was reloaded. On any refusal
  (reserved or duplicate id, executable not on PATH, invalid definition) nothing is written and
  the exit code is not 0.
";

pub const LIST: &str = "\
many-ai-cli provider list

One line per AI CLI, tab-separated: id, display name, origin (built-in, legacy or user),
enabled or disabled, headless yes or no. \"legacy\" means defined in config.yaml
custom_providers; \"user\" means registered with `provider add` or the dashboard.
";

pub const REMOVE: &str = "\
many-ai-cli provider remove <id>

Removes an AI CLI that was registered with `provider add` or the dashboard. Built-in CLIs and
those defined in config.yaml (legacy) are refused with a message. There is no confirmation
prompt. With a Hub running the removal goes through it (backup, reload, dashboards updated);
a backup of the removed definition is kept either way:
  many-ai-cli provider backup list <id>
";

pub const TEST: &str = "\
many-ai-cli provider test <id> [--prompt <text>] [--timeout <sec>]

Runs the provider's headless definition once and prints the exit code, the seconds taken and the
first 10 lines of output. The default prompt asks for the answer to 1+1; the default timeout is
120 seconds, after which the process is killed. The process gets no Hub credentials. Fails with a
message when the provider has no headless definition, when its executable is not on PATH, and
(with exit code not 0) when the run fails or times out.
";
