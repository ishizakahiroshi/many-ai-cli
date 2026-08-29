# One Skills Shelf and One Canonical Rule File Across AI CLIs

> **This file is the canonical copy of the procedure.** It is written to be handed
> straight to an AI agent: open a session in any of the CLIs below and say
> `<path or URL to this file>, do this`. Everything an agent needs is written
> out here, so no follow-up prompting should be required.
>
> To hand it over by URL (returns raw Markdown):
> `https://raw.githubusercontent.com/ishizakahiroshi/many-ai-cli/main/docs/manual_shared-skills-and-rules.md`
>
> A narrative version with diagrams, **in Japanese**, lives at
> <https://ishizakahiroshi.com/articles/2026/2026-08-30_multi-ai-cli-shared-skills-and-rules/>.
> It links back here rather than repeating the procedure.
>
> The wiring described here is independent of `many-ai-cli`. It works with or
> without the Hub; it just pairs well with running several CLIs side by side.

---

## 1. Why

If you use Claude Code, Codex, OpenCode, GitHub Copilot CLI, Grok and Cursor Agent
together, you end up maintaining the same thing twice.

- **Skills** (Agent Skills format, `<name>/SKILL.md`) scatter into a different
  directory per CLI.
- **Shared rules** (coding conventions, things you do not want the agent to do)
  get copied once per CLI.

Both problems have the same fix: keep the canonical copy in one place, and let each
CLI reach it in whatever way that CLI expects.

- For skills, put the directory in **one** place and link to it from each CLI's
  search path.
- For rules, make **one** file canonical and have every other global instruction
  file point at it.

This document covers building those two things.

### The underlying annoyance: every AI reads a different filename

You want to state one working rule, but the receiving side has no agreed-on name.

```
CLAUDE.md   AGENTS.md   GEMINI.md   copilot-instructions.md   .cursor/rules   CONTEXT.md
```

The locations differ too. The obvious move, "just write all of them", leads here:

```
[the painful shape]                        [the shape you want]

CLAUDE.md  146 lines of rules              AGENTS.md   1 line: "read CLAUDE.md"  ─┐
AGENTS.md  146 lines of rules (same)       GEMINI.md   1 line: "read CLAUDE.md"  ─┼→ CLAUDE.md
GEMINI.md  146 lines of rules (same)       copilot-instructions.md  1 line       ─┘  146 lines of rules
                                                                                     edited only here
Every added rule means editing 3 files.    You edit one file.
Forget one and the CLIs disagree.          The others stay one line forever and cannot drift.
You also need a check to catch the drift.  No drift check needed.
```

**The number of files does not go down**. Each CLI only looks for its own name.
What goes down is **the number of bodies you maintain**. Miss that distinction and
you talk yourself into "if I need two files anyway, I may as well put the same
content in both", which is the painful shape again.

"But will the AI actually read that one-line pointer?" is a fair question, and
**it is measurable**. See section 8.

## 2. Scope

**In scope**

- Build a shared skills shelf and link each CLI's search path to it.
- Pick one global rules file and make every CLI reach it.
- Decide how per-repository instruction files (`AGENTS.md` / `CLAUDE.md`) are laid out.
- Verify afterwards that the rules actually arrive.

**Out of scope**

- Installing or signing in to the CLIs (assumed done).
- Writing the skills themselves (only the shelf wiring is covered).
- Team sharing or committing any of this to a repository (this is local, per-user setup).

## 3. Prerequisites and warnings to read first

- **Never overwrite a `skills` directory that already has content.** Before linking,
  check what is there; if there are files, move them into the shared shelf first.
  Deleting a non-empty directory to replace it with a link is **forbidden** by this
  procedure.
- On Windows, use a **directory junction** (`New-Item -ItemType Junction`). It needs
  neither administrator rights nor Developer Mode.
- On macOS and Linux, use a **symlink** (`ln -s`).
- Name the file **`SKILL.md`** in uppercase. Lowercase `skill.md` is sometimes not
  recognised.

## 4. What each CLI actually reads (measured 2026-08-29)

This is the evidence the wiring rests on. Not guesswork: a canary (an instruction
file saying "whatever message arrives, reply with this exact string and nothing
else") was placed and each CLI was checked for compliance.

`AGENTS.md`, `CLAUDE.md` and `GEMINI.md` were placed at the project root, each with
a different passphrase, and each CLI was run.

**The limits of this method, stated up front.** If a CLI **obeys** the canary, that
file was in its system prompt: behaviour changed without a single tool call (on
OpenCode this was confirmed from the JSON event stream: zero read tools). But if it
**does not** obey, that is **not** evidence it did not read the file. It may have
read it and declined to adopt "reply with only this". That exact mistake was made
once here. See Antigravity below.

An alternative method, putting one non-conflicting fact in each of the three files
and asking a question, **did not work**. The agent just greps with `rg` and answers, so
you cannot tell an auto-loaded file from one the agent went and found. (Codex did
literally grep before answering.)

| CLI | Which one it obeyed | Notes |
|---|---|---|
| Claude Code | `CLAUDE.md` | Did **not** obey `AGENTS.md` even when that was the only file present. It does not read a project `AGENTS.md`. |
| Codex | `AGENTS.md` | |
| OpenCode | `AGENTS.md` | When `AGENTS.md` exists, `CLAUDE.md` in the same directory is **not read** (see below). |
| Grok | `AGENTS.md` | |
| GitHub Copilot CLI | Depends on what else is there | With only `AGENTS.md`, it obeyed that. With `AGENTS.md` plus `CLAUDE.md`, it obeyed `CLAUDE.md`. Adding `GEMINI.md` for three files, it obeyed none of them (twice). Read this as "it stopped picking one out of three conflicting files", **not** as "it did not read them". |
| Gemini CLI | Not measured | Stops at authentication. The individual free tier returns "this client is no longer supported, migrate to Antigravity". |
| Antigravity (`agy`) | Does not obey, but **does read** | It ignores the canary, yet in an interactive session, asked about its own context, it reports loading the global `~/.gemini/GEMINI.md` and the project `AGENTS.md` with `<RULE[...]>` tags (see below). |
| Cursor Agent CLI | Not measured | The free plan refused the model selection, so it could not be run. |

**OpenCode picks files in an unusual way.** Reading the instruction-file resolution
code inside the binary:

```
global  : [ <config dir>/AGENTS.md , ~/.claude/CLAUDE.md ]
          read the first one that exists, then break

project : [ AGENTS.md , CLAUDE.md , CONTEXT.md ]
          read the first kind found, then break
```

The global and project levels are merged, but **within one level the first match
wins and excludes the rest**. Two consequences:

- Creating `<config dir>/AGENTS.md` stops the fallback to `~/.claude/CLAUDE.md`.
  **Better not to create it.**
- If a project has `AGENTS.md`, that repository's `CLAUDE.md` never reaches OpenCode.

### Antigravity was reading it (an example of measuring the wrong thing)

Because it did not obey the canary, the first conclusion was "it does not read
`AGENTS.md`". That was wrong. Asked in an interactive session what it loaded at
startup, it answers:

```
Scope                     Loaded file              Tag in the prompt
Global (whole machine)    ~/.gemini/GEMINI.md      <RULE[user_global]>
Project (workspace)       AGENTS.md                <RULE[<project path>/AGENTS.md]>
```

It also explains the discovery rule: **walk up from the current directory to the
repository root looking for `GEMINI.md`, `AGENTS.md` and `.agents/rules/*.md`.** The
global `~/.gemini/GEMINI.md` is auto-loaded.

The decisive detail is that it names the internal prompt tags. Merely finding a file
on disk would not produce those. **Loading and obeying are different things**, and
the canary only measures the second.

Source: responses from an interactive session of Antigravity CLI 1.1.22. Not vendor
documentation.

**Global instruction files live in a different place for every CLI.** There is no
single path that covers them all.

| CLI | Global instruction file |
|---|---|
| Claude Code | `~/.claude/CLAUDE.md` |
| OpenCode | `<config dir>/AGENTS.md`; falls back to `~/.claude/CLAUDE.md` if absent |
| GitHub Copilot CLI | `~/.copilot/copilot-instructions.md`. The official docs also mention `.claude/CLAUDE.md` (not measured here) |
| Codex | `~/.codex/AGENTS.md` |
| Grok | `AGENTS.md` and similar under `~/.grok/` |
| Gemini CLI | `~/.gemini/GEMINI.md` |
| Antigravity (`agy`) | `~/.gemini/GEMINI.md` (the same file as Gemini CLI) |
| Cursor Agent CLI | None. Global user rules are not available from the CLI |

## 5. Part A: build the shared skills shelf

### A-1. Choose where the canonical shelf lives

Anywhere is fine, as long as it is **outside** any CLI's config directory. Below it
is referred to as:

- Windows: `%USERPROFILE%\dev\ai-shelf\skills`
- macOS / Linux: `~/dev/ai-shelf/skills`

Create it if it does not exist. If some CLI already holds skills, **move** them here
(move, not copy; do not leave a second copy behind).

### A-2. Inspect the existing directories

Before linking, confirm each target is either missing or empty. If it has content,
move that into the shelf from A-1 first.

Windows:

```powershell
$paths = @('.claude\skills','.agents\skills','.cursor\skills','.config\opencode\skills','.copilot\skills','.gemini\config\skills')
foreach ($rel in $paths) {
  $p = Join-Path $env:USERPROFILE $rel
  if (Test-Path $p) {
    $item = Get-Item $p -Force
    $count = @(Get-ChildItem $p -Force -ErrorAction SilentlyContinue).Count
    "{0,-28} type={1,-10} entries={2}" -f $rel, ($item.LinkType ?? 'dir'), $count
  } else { "{0,-28} missing" -f $rel }
}
```

macOS / Linux:

```bash
for rel in .claude/skills .agents/skills .cursor/skills .config/opencode/skills .copilot/skills .gemini/config/skills; do
  p="$HOME/$rel"
  if [ -e "$p" ]; then
    printf '%-28s %s entries=%s\n' "$rel" "$( [ -L "$p" ] && echo symlink || echo dir )" "$(ls -A "$p" 2>/dev/null | wc -l)"
  else
    printf '%-28s missing\n' "$rel"
  fi
done
```

**If even one of them reports a non-zero entry count, stop and ask the user.** Do not
delete anything on your own.

### A-3. Create the links

Windows (junctions; written to skip anything that already exists):

```powershell
$src = Join-Path $env:USERPROFILE 'dev\ai-shelf\skills'
if (-not (Test-Path $src)) { New-Item -ItemType Directory -Path $src -Force | Out-Null }
foreach ($rel in @('.claude\skills','.agents\skills','.cursor\skills','.config\opencode\skills','.copilot\skills','.gemini\config\skills')) {
  $p = Join-Path $env:USERPROFILE $rel
  if (Test-Path $p) { "skip (already present): $rel"; continue }
  $parent = Split-Path $p -Parent
  if (-not (Test-Path $parent)) { New-Item -ItemType Directory -Path $parent -Force | Out-Null }
  New-Item -ItemType Junction -Path $p -Target $src | Out-Null
  "created: $rel"
}
```

macOS / Linux (symlinks):

```bash
src="$HOME/dev/ai-shelf/skills"
mkdir -p "$src"
for rel in .claude/skills .agents/skills .cursor/skills .config/opencode/skills .copilot/skills .gemini/config/skills; do
  p="$HOME/$rel"
  if [ -e "$p" ]; then echo "skip (already present): $rel"; continue; fi
  mkdir -p "$(dirname "$p")"
  ln -s "$src" "$p"
  echo "created: $rel"
done
```

### A-4. Places you must not touch

- **Do not link `~/.codex/skills`.** It holds the system skills bundled with Codex;
  replacing the whole directory hides them. Codex reaches your shelf through
  `~/.agents/skills` instead.
- **Do not link `~/.gemini/antigravity-cli/builtin/skills` either.** That holds the
  skills bundled with Antigravity. The user shelf goes to `~/.gemini/config/skills`.

### A-5. If you only use three CLIs

For Codex + Claude Code + OpenCode you need three links:

- `~/.claude/skills` (Claude Code)
- `~/.agents/skills` (Codex)
- `<config dir>/opencode/skills` (OpenCode; on most systems `~/.config/opencode/skills`)

Add `~/.gemini/config/skills` if you also use Antigravity (next section).

### A-6. Wiring Antigravity (`agy`)

The bundled skill `agy-customizations`
(`~/.gemini/antigravity-cli/builtin/skills/agy-customizations/`) is the specification
of record. Discovery has three tracks.

| Scope | Location | Notes |
|---|---|---|
| Workspace | `.agents/` (also `.agent/` / `_agents/` / `_agent/`) | Searched by walking up from cwd to the repository root |
| Hierarchical rules | `GEMINI.md` / `AGENTS.md` / `.agents/rules/*.md` | Read the same way, walking up |
| Global | `~/.gemini/config/` | The `skills/` directory under it is the user shelf |

Skills use the same `skills/<name>/SKILL.md` shape as the other CLIs, so the shelf
connects directly.

```powershell
New-Item -ItemType Junction `
  -Path (Join-Path $env:USERPROFILE '.gemini\config\skills') `
  -Target <path to the shared shelf>
```

For rules, `~/.gemini/GEMINI.md` is auto-loaded (the same file as Gemini CLI). Put
the pointer line to your canonical file there and it arrives.

Verify in two stages. **Appearing in a listing and actually being loaded are
different things.**

```
# 1. Is it visible? (should match the number of directories in the shelf)
agy --print "How many skills do you have available right now?"

# 2. Does it actually load? (name one skill from the shelf)
agy
> can you do <name of a skill in the shelf>?
```

Success on step 2 looks like `Read(~/.gemini/config/skills/<skill>/SKILL.md)` in the
response, followed by an explanation grounded in that skill's text. That proves it
followed the junction through to the real files.

## 6. Part B: make one canonical rules file

### B-1. Pick the canonical file

**`~/.claude/CLAUDE.md` is the advantageous choice.** Per the table in section 4,
Claude Code reads that path natively, OpenCode reads it as a fallback, and Copilot's
documentation says it uses it too. It has the widest reach at the global level.

Moving the global file to `AGENTS.md` because "`AGENTS.md` is the mainstream name"
**buys you nothing.** Global locations differ per CLI, so unifying the *name* still
does not give you one file. It gives you three or more copies.

### B-2. CLIs that only get a pointer

In `~/.codex/AGENTS.md` and `~/.grok/AGENTS.md`, put a single line that sends the
agent to the canonical file.

```markdown
# Shared rules

Before starting work, read `~/.claude/CLAUDE.md` and follow the shared rules there.
This file holds only CLI-specific notes.
```

**This pointer is not an auto-load.** It only arrives once the AI opens the file
itself. Whether it does arrive is measurable. See section 8.

### B-3. Files you must not create

- **Do not create `<config dir>/opencode/AGENTS.md`.** Creating it stops OpenCode's
  fallback to `~/.claude/CLAUDE.md` (the first-match-wins behaviour in section 4).

### B-4. Skeleton for the canonical file

The content is up to you, but **making it an index and pushing the prose into
separate files** keeps it from eating every CLI's context on every session. A
skeleton:

```markdown
# Shared rules (canonical, for every AI CLI)

> This file is an index. Read what each line points at. Do not write the prose back in here.

## Choosing tools
(How to pick a shell, preferring dedicated tools over shell commands, and so on; one line each.)

## Things not to do
- Do not build unless asked (`go build`, `npm run build`, ...)
- Do not commit, push or tag unless asked
- Do not dump files that mix config with credentials (`.env`, `.npmrc`, ...)

## Output format
(Response formatting, how to write paths, and so on.)

## Index of detailed guides
| When to read it | File |
|---|---|
| Doing a release | `~/.claude/guides/<name>.md` |
```

**Add one mechanism that stops the index from growing.** Even a script that just
checks the line count, wired into CI or a pre-commit hook, is enough to stop the
prose creeping back in.

### B-5. Per-repository instruction files

At a repository root, the easiest arrangement is to make `CLAUDE.md` canonical and
put a pointer in `AGENTS.md`. Per section 4, Claude Code does not read `AGENTS.md`
while Codex and OpenCode do, so this blocks neither entrance.

Example `AGENTS.md`:

```markdown
# Agent Entry Point

The working rules for this repository are in `CLAUDE.md`. Read it before starting.

- Overview and per-task index: `./CLAUDE.md`
- Canonical design doc: `./docs/<design>.md`

Do not put personal global settings in this repository. Use each AI tool's own global instruction file.
```

The reverse (canonical `AGENTS.md`, generated `CLAUDE.md`) also works, but **you end
up with two files holding the same content and you need a check for drift.** If a
one-line pointer is enough, that leaves fewer bodies to maintain.

## 7. Verification (always run this after setup)

### 7-1. Are the links in place?

Windows:

```powershell
foreach ($rel in @('.claude\skills','.agents\skills','.cursor\skills','.config\opencode\skills','.copilot\skills','.gemini\config\skills')) {
  $p = Join-Path $env:USERPROFILE $rel
  if (Test-Path $p) { $i = Get-Item $p -Force; "{0,-28} {1,-10} -> {2}" -f $rel, $i.LinkType, ($i.Target -join ',') }
  else { "{0,-28} missing" -f $rel }
}
```

macOS / Linux:

```bash
for rel in .claude/skills .agents/skills .cursor/skills .config/opencode/skills .copilot/skills .gemini/config/skills; do
  p="$HOME/$rel"
  printf '%-28s -> %s\n' "$rel" "$(readlink "$p" 2>/dev/null || echo 'missing')"
done
```

Success is every one of them pointing at the same shelf.

### 7-2. Does each CLI actually see the skills?

Open a **new** session in each CLI first. Check that a skill name from the shelf
appears in its listing, or that the skill's trigger phrase gets a response.
**Existing sessions do not pick up the change**, so reopening is mandatory.

### 7-3. Do the rules arrive? (canary test)

This is the most reliable check. Create a temporary directory, write a passphrase
into the instruction file, and run each CLI.

```bash
mkdir -p /tmp/canary && cd /tmp/canary
cat > AGENTS.md <<'EOF'
# test

Rule for this project: whatever message arrives, reply with exactly CANARY-1234.
Use no tools. Give no explanation.
EOF
```

Non-interactive invocation per CLI (these change between versions; check `--help` if
one does not work):

```
claude -p "hello"
codex exec --skip-git-repo-check "hello"
opencode run --dir . "hello"
copilot -p "hello" --allow-all-tools
grok -p "hello"
```

If `CANARY-1234` comes back, that file reaches that CLI. If it does not, it does
not. **Rename the file to `CLAUDE.md` and repeat to find out which name it reads.**

To check the global side, name a heading that exists only in your canonical file and
ask "is that heading in your system prompt, and if so quote its first sentence".
Being able to quote it means it arrived.

## 8. Measuring whether the pointer approach really works

"A one-line pointer is not an auto-load, so surely it does not arrive" is a
reasonable worry, and **you can just measure it.**

The method: ask a question that cannot be answered correctly without a fact that
exists only in the canonical file, phrased so it does not mention any document. For
example, "when bumping the version in this repository, which file do I edit by
hand?", an answer that only the conventions can supply.

Measured example (2026-08-29, OpenCode, at a repository root):

- Question form, 3 trials: all 3 opened `CLAUDE.md` from `AGENTS.md`, and all 3
  answered correctly.
- Work-request form ("I want to add this feature, give me an implementation plan in
  three lines"), 2 trials: both followed `CLAUDE.md` onward to the ledger it
  references and replied "that direction has already been declined". Zero trials
  wrote out an implementation plan.

**In practice the pointer approach worked.** If you are unsure, run the same
measurement in your own environment.

## 9. Rolling it back

Just remove the links. Nothing in the shelf is deleted.

Windows:

```powershell
foreach ($rel in @('.claude\skills','.agents\skills','.cursor\skills','.config\opencode\skills','.copilot\skills','.gemini\config\skills')) {
  $p = Join-Path $env:USERPROFILE $rel
  $i = Get-Item $p -Force -ErrorAction SilentlyContinue
  if ($i -and $i.LinkType -eq 'Junction') { Remove-Item $p -Force; "removed: $rel" }
  else { "skip (not a link): $rel" }
}
```

macOS / Linux:

```bash
for rel in .claude/skills .agents/skills .cursor/skills .config/opencode/skills .copilot/skills .gemini/config/skills; do
  p="$HOME/$rel"
  if [ -L "$p" ]; then rm "$p"; echo "removed: $rel"; else echo "skip (not a link): $rel"; fi
done
```

**Never delete something that is not a link**. Deleting a real directory takes its
contents with it.

## 10. Stop conditions (for AI agents)

If any of the following happens, stop and ask the user instead of continuing.

- The check in 5/A-2 found a **non-empty directory** where a link was going to go.
- You are asked to touch `~/.codex/skills`.
- You would have to **overwrite** an existing `~/.claude/CLAUDE.md` or
  `~/.codex/AGENTS.md` (appending is fine, replacing is not).
- Some CLI did not return the expected passphrase during the section 7 verification.

## 11. Not measured, and environment differences

- Cursor Agent CLI was not measured (the free plan refused the model selection). It
  is said to read a project's `AGENTS.md` / `CLAUDE.md` / `.cursor/rules`; run the
  canary test from 7-3 yourself to confirm.
- Whether Copilot CLI reads the global `~/.claude/CLAUDE.md` was not measured (based
  on a report that the official docs mention it). On the project side, the result
  changed with how many instruction files were present. **If you keep several
  instruction files side by side, do not let them contradict each other on Copilot.**
- Gemini CLI could not be run. The individual free tier answers "this client is no
  longer supported, migrate to Antigravity". It is said to read `~/.gemini/GEMINI.md`,
  but that is unverified here.
- Antigravity's discovery behaviour comes from the bundled `agy-customizations` skill
  (being a bundled artifact, that is stronger than self-report, but it was not
  cross-checked against public documentation). What it loads is self-reported from an
  interactive session.
- Grok is reported to have a per-file read limit. That is one more reason to point at
  shared rules rather than copy them, but the limit itself is unverified.
- Discovery behaviour changes between versions. **Do not take the tables here on
  faith. Running the canary test from 7-3 against your own environment is the
  fastest way to know.**
