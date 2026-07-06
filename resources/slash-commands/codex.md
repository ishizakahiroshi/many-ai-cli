# Codex Slash Commands

| Command | Purpose | When to use it |
|---|---|---|
| `/agent` | Switch the active agent thread. | Inspect or continue work in a spawned subagent thread. |
| `/apps` | Browse apps and insert them into your prompt. | Attach an app before asking Codex to use it. |
| `/approve` | Approve one retry of a recent auto-review denial. | Retry an action that the automatic reviewer denied. |
| `/clear` | Clear the terminal and start a fresh chat. | Reset the UI and conversation together. |
| `/compact` | Summarize the visible conversation to free tokens. | After long runs, to retain key points while reclaiming context. |
| `/copy` | Copy the latest completed Codex output. | Grab finished responses without manual selection. |
| `/debug-config` | Print config layer and requirements diagnostics. | Debug config precedence and policy requirements. |
| `/diff` | Show the Git diff including untracked files. | Review edits before committing or testing. |
| `/exit` | Exit the CLI. | Leave the session immediately. |
| `/experimental` | Toggle experimental features. | Enable optional features such as subagents. |
| `/fast` | Toggle the Fast service tier. | Turn the current model's Fast tier on or off. |
| `/feedback` | Send logs to Codex maintainers. | Report issues or share diagnostics. |
| `/fork` | Fork the current conversation into a new thread. | Branch the session to explore alternatives. |
| `/goal` | Set, pause, resume, view, or clear a task goal. | Give Codex a persistent target to track across turns. |
| `/hooks` | Review lifecycle hooks. | Inspect hook configuration and manage hook trust. |
| `/ide` | Include open files and the current IDE selection. | Pull editor context into the next prompt. |
| `/init` | Generate an `AGENTS.md` scaffold. | Capture persistent repository instructions. |
| `/keymap` | Remap TUI keyboard shortcuts. | Inspect and persist custom shortcut bindings. |
| `/logout` | Sign out of Codex. | Clear credentials on shared machines. |
| `/mcp` | List configured Model Context Protocol tools. | Check which external tools Codex can call. |
| `/memories` | Configure memory use and generation. | Turn memory injection or generation on or off. |
| `/mention` | Attach a file to the conversation. | Point Codex at specific files to inspect. |
| `/model` | Choose the active model and reasoning effort. | Switch models or reasoning effort before running a task. |
| `/new` | Start a new conversation in the same CLI session. | Reset chat context without leaving the terminal. |
| `/permissions` | Set what Codex can do without asking first. | Relax or tighten approval requirements mid-session. |
| `/personality` | Choose a communication style for responses. | Make Codex more concise, explanatory, or collaborative. |
| `/plan` | Switch to plan mode and optionally send a prompt. | Request an execution plan before implementation work. |
| `/plugins` | Browse installed and discoverable plugins. | Inspect tools, install suggested plugins, or manage availability. |
| `/ps` | Show background terminals and their status. | Check long-running commands without leaving the transcript. |
| `/quit` | Exit the CLI. | Leave the session immediately. |
| `/raw` | Toggle raw scrollback mode. | Make terminal selection and copying more direct. |
| `/resume` | Resume a saved conversation from the session list. | Continue work from a previous CLI session. |
| `/review` | Ask Codex to review your working tree. | Run after completion or for a second opinion on changes. |
| `/sandbox-add-read-dir` | Grant sandbox read access to an extra directory. | Unblock reads for a directory outside the current readable roots. |
| `/side` | Start an ephemeral side conversation. | Ask focused follow-ups without disrupting the main thread. |
| `/skills` | Browse and use skills. | Inspect bundled or installed skills for the current task. |
| `/status` | Display session configuration and token usage. | Confirm active model, policy, and context capacity. |
| `/statusline` | Configure TUI status-line fields interactively. | Pick and reorder footer items. |
| `/stop` | Stop all background terminals. | Cancel background terminal work. |
| `/theme` | Choose a syntax-highlighting theme. | Preview and persist a terminal syntax theme. |
| `/title` | Configure terminal title items interactively. | Pick and reorder title items. |
| `/vim` | Toggle Vim mode for the composer. | Switch between Vim and default editing behavior. |
