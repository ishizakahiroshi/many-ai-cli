//! `--help` text for `many-ai-cli provider ...`. The `add` text is written for an
//! AI agent that was asked to register a new AI CLI: it says what to research,
//! what to show the user, what the user (not the agent) answers, and how to
//! check the result.

pub const OVERVIEW: &str = "\
many-ai-cli provider - register, list, remove and test AI CLIs

USAGE
  many-ai-cli provider add --id <id> --command <executable> [options]   register a new AI CLI
  many-ai-cli provider add --definition <file.json> [options]           register one from a definition file
  many-ai-cli provider list                                             list every AI CLI
  many-ai-cli provider remove <id>                                      remove one you registered
  many-ai-cli provider test <id> [--prompt <text>] [--timeout <sec>]    run its headless mode once, then list what it can do
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
  many-ai-cli provider add --definition <file.json> [options]

OPTIONS
  --definition <file>       a whole definition as JSON (relative to the current folder). Best for an
                            AI agent: write the file once, with every item you found. --id,
                            --command and the options below, when given, replace the file's values.
                            Without --definition, --id and --command are required.
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
  --model <id>              a model the dashboard offers for it; repeat
  --letter <text>           the 1-2 character icon badge
  --color <#rrggbb>         the icon badge color
  --usage-url <https url>   its usage / billing page (linked from Settings)
  --slash-source <source>   where its slash command list comes from (an https raw.githubusercontent.com
                            URL of a Markdown table, or an absolute file under the many-ai-cli home)
  --permission-mode <name>=<arg>[,<arg>...]
                            a permission mode and the launch arguments it adds; repeat. The names
                            become the choices of the new-session form.
  Giving any --headless-arg, --headless-format or --prompt-via makes the CLI usable as an
  `orchestrate spawn --execution-mode headless` child.

  A value that starts with '-' is fine: --arg --interactive, --headless-arg -p.

IF YOU ARE AN AI AGENT ASKED TO ADD A NEW CLI
  Do not guess. Find these in the CLI's own --help and official documentation, and show the user
  what you found before you register it. For every item say where it came from (the official
  documentation page, the CLI's --help, or \"not found, so not set\"). A value you could not
  find is left out of the definition: a missing item is shown to the user as \"unavailable\" with
  the reason, while a guessed one breaks silently.
  1. The command that starts the interactive UI (this becomes --command and --arg).
  2. Its non-interactive flag (the --headless-arg values) and whether the prompt is passed as an
     argument or on stdin (--prompt-via).
  3. Its model flag (launch.model_args, e.g. [\"--model\", \"{model}\"]) and the command that lists
     its models, if either exists (models.list.args, models.items or --model).
  4. Its permission modes and the launch flag of each (--permission-mode, permissions.modes).
  5. Where its slash commands are documented (--slash-source, or slash_commands.items for a short
     fixed list).
  6. Where it saves its conversations (transcript.root, a pattern starting with {home} or
     {env:NAME}), the file format (transcript.reader) and how a session's file is found
     (transcript.find). Use this only after you have looked at a real saved file.
  7. The exact text its approval prompts print and the key that answers each one. Copy the option
     strings verbatim into --approval-option; the keys go in the definition's approval section
     (style, keys). Without them only the generic wording (Yes/No/Allow/Deny) is detected.
  8. Its usage page URL (--usage-url) and the argument that starts its login (login.args).
  9. Where it keeps its home or config. If it needs a different HOME or a config directory,
     write a small wrapper script that sets it and registers the script as --command.
  10. Whether the CLI's terms of service allow another tool to drive it. Report what the terms
     say and where; the user decides. If the user says no, do not register it. Do not decide
     this for the user.
  Not declarable (shown as \"unsupported\"): reading the remaining quota, switching subscription
  profiles and the sub-agent tree need code written for that vendor. Do not try to fake them.
  A CLI that is reached over ACP (a protocol the CLI speaks on its own, not a terminal) is driven
  by a person only: the user registers and starts it from the dashboard. Do not set
  \"connection\": \"acp\" yourself, do not use it as a child, relay, routine or board target, and
  never extract or relay its sign-in token. A sample for Antigravity is docs/samples/acp-agent-sample.json
  in the repository (add it only when the user asks: provider add --definition <file>). Report what
  the vendor's current terms say and the URL; the user decides whether to use it.
  Things you must not do: print, read aloud or ask for the Hub token (the command reads it
  itself); answer a first-run \"do you trust this folder\" or a login prompt of the new CLI
  (the user answers those in the dashboard session); keep retrying with different flags if
  `provider test` fails - show the user the output and ask.

  No approval from the user is awaited when you run this command. With a Hub running, the
  dashboard shows \"<name> was added (command: ...)\" with an Undo, so the user keeps control
  without an extra step. Tell the user what you added and the command line that will run.

AFTER ADDING
  Run `many-ai-cli provider test <id>`: it runs the headless mode once with a small prompt, prints
  the exit code, the seconds taken and the first lines of output, then lists what the CLI can do
  (available / unavailable with the reason / unsupported). Read the list back to the user and say
  which unavailable items you could not find. `provider list` shows the result; `provider remove
  <id>` undoes it.

OUTPUT
  On success: the id, the exact executable and arguments that will run (interactive and
  headless), whether headless is available, the list of what it can do, and whether a Hub was
  reloaded. On any refusal
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
first 10 lines of output, and then lists what the provider can do, one line per feature:
  available     declared (and, for the slash command source, the conversation folder and the
                model list command, tried just now)
  unavailable   not declared or the check failed; the reason says what to add
  unsupported   needs code written for that vendor, so it cannot be declared
The default prompt asks for the answer to 1+1; the default timeout is
120 seconds, after which the process is killed. The process gets no Hub credentials. Fails with a
message when the provider has no headless definition, when its executable is not on PATH, and
(with exit code not 0) when the run fails or times out.
";
