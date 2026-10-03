#!/usr/bin/env python3
"""Regenerate the K01 source contract, never application implementation coverage.

The oracle helpers run separately, with the isolated Go environment documented in
rust/tests/fixtures/foundation/cli/README.md. This script refuses stale goldens.
"""
import argparse
import hashlib
import json
import pathlib
import re

ROOT = pathlib.Path(__file__).resolve().parents[2]
FIXTURES = ROOT / "rust/tests/fixtures/foundation/cli"
BASELINE = "21d0bc7935a2c4696fb89ccff2e324157a528c2d"


def read(path):
    return (ROOT / path).read_text()


def evidence(path, anchor, test=None):
    text = read(path)
    position = text.find(anchor)
    if position < 0:
        raise ValueError(f"missing source anchor {path}: {anchor}")
    item = {"source": path, "line": text[:position].count("\n") + 1, "anchor": anchor}
    if test:
        test_path, test_name = test.split("::", 1)
        if not re.search(r"func " + re.escape(test_name) + r"\(", read(test_path)):
            raise ValueError(f"missing test {test}")
        item["baseline_test"] = test
    return item


def contract(identifier, statement, source, anchor, test=None):
    return {"id": identifier, "contract": statement, "evidence": evidence(source, anchor, test)}


def generate():
    lexical = json.loads((FIXTURES / "source-oracle.json").read_text())
    pure = json.loads((FIXTURES / "pure-oracle.json").read_text())
    for oracle in (lexical, pure):
        for source in oracle["sources"]:
            actual = hashlib.sha256((ROOT / source["path"]).read_bytes()).hexdigest()
            if source["sha256"] != actual:
                raise ValueError(f"stale CLI oracle: {source['path']}; regenerate the Go oracle first")
    main = "cmd/many-ai-cli/main.go"
    provider = "cmd/many-ai-cli/provider_command.go"
    orchestrate = "internal/orchestrate/orchestrate.go"
    wrapper = "internal/wrapper/wrapper.go"
    relay = "internal/usagerelay/usagerelay.go"
    launcher = "cmd/many-ai-cli-launcher/main.go"
    issue = "cmd/many-ai-cli/issue.go"
    builtins = next(row["names"] for row in lexical["dispatch"] if row["names"][:1] == ["claude"])
    usage = re.search(r'fmt.Println\("(many-ai-cli <[^"\n]+>)"\)', read(main))[1]
    commands = []
    for row in lexical["dispatch"]:
        for name in row["names"]:
            if name in builtins:
                route = "wrap"
            elif name in ("version", "--version", "-v"):
                route = "version"
            elif name in ("help", "--help", "-h"):
                route = "help"
            else:
                route = name
            commands.append({
                "name": name, "binary": "many-ai-cli", "route": route,
                "alias_of": route if name != route else None,
                "hidden": name in ("usage-relay", "orchestrate"),
                "advertised_in_usage": name in usage.removeprefix("many-ai-cli <").removesuffix(">").split("|"),
                "source": main, "line": row["line"], "go_calls": row["calls"],
                "rust_dispatch": "rust/src/cli.rs::parse",
                "parse_status": "top-level route covered by source-derived dispatch fixture; delegated parsing is separate",
                "runtime_status": "pending",
                "runtime_note": "candidate help/version output exists; pre-dispatch config load and version lookup parity pending" if route in ("help", "version") else "not integrated by the current binary; parsing is not runtime acceptance",
            })
    commands += [
        {"name": "<no arguments>", "binary": "many-ai-cli", "route": "default", "source": main,
         "go_calls": ["config.LoadOrCreate", "hub.IsRunning", "hub.OpenBrowserForConfig or hub.NewServer.Run"],
         "parse_status": "top-level route covered", "runtime_status": "pending"},
        {"name": "<configured custom provider ID>", "binary": "many-ai-cli", "route": "wrap", "source": main,
         "go_calls": ["cfg.IsCustomProviderID", "wrapper.Run"],
         "parse_status": "route covered with an explicit synthetic custom-provider list; binary config wiring pending",
         "runtime_status": "pending", "precedence": "only after every built-in command/alias case"},
        {"name": "many-ai-cli-launcher", "binary": "many-ai-cli-launcher", "source": launcher,
         "rust_dispatch": "rust/src/cli.rs::parse_launcher", "parse_status": "standalone Go flag grammar covered",
         "runtime_status": "pending", "go_calls": ["launcher.ConfigureConsoleUTF8", "launcher.StartupBanner", "launcher.LoadProfiles", "launcher.Validate", "runUI or launcher.SelectProfile + launcher.Connect"]},
    ]
    subcommands = []
    def sub(name, path, function, syntax, behavior, flagset=None):
        subcommands.append({"name": name, "syntax": syntax, "go_entry": function,
                            "evidence": evidence(path, "func " + function + "("),
                            "flag_set": flagset, "behavior": behavior,
                            "parse_status": "opaque top-level argv forwarded; source contract recorded for downstream implementation",
                            "runtime_status": "pending"})
    sub("provider", provider, "runProviderCommand", "provider <backup|reset|recover>", "Missing verb errors before config.Dir; nonempty verb resolves config and history store before branch validation. No help alias.")
    sub("provider backup", provider, "runProviderBackupCommand", "provider backup <list|verify|restore> <provider-id> [backup-id]", "Requires at least action and provider ID. Unknown action errors; trailing argv is tolerated.")
    sub("provider backup list", provider, "runProviderBackupCommand", "provider backup list <provider-id>", "ListBackups; one tab-separated revision, created_at, reason, content_digest row per backup. Extra args ignored.")
    sub("provider backup verify", provider, "runProviderBackupCommand", "provider backup verify <provider-id> <backup-id>", "VerifyBackup; stdout verified TAB revision TAB digest. Extra args ignored.")
    sub("provider backup restore", provider, "runProviderBackupCommand", "provider backup restore <provider-id> <backup-id> --expected-revision <revision>", "Manual scan starts at argv index 3; only separate --expected-revision recognized; last complete occurrence wins; empty or missing value rejected; unknown args ignored.")
    sub("provider reset", provider, "runProviderResetCommand", "provider reset --distributed <provider-id> --expected-revision <revision>", "First arg must be --distributed. Exact built-in IDs only. Presence of a separate expected-revision value required, but empty value accepted; last complete occurrence wins.")
    sub("provider recover", provider, "runProviderRecoverCommand", "provider recover <provider-id> [revision|--list]", "No revision requests base recovery; any second arg other than --list is a revision, including unknown flags; extra args ignored. A healthy HEAD is rejected by HistoryStore.")
    sub("provider recover --list", provider, "runProviderRecoverCommand", "provider recover <provider-id> --list", "Only argv index 1 selects --list; prints optional candidate TAB user_revision TAB revision TAB created_at then candidate TAB base; read-only.")
    sub("orchestrate", orchestrate, "Run", "orchestrate <spawn|send|relay>", "Main missing-verb error is older <spawn|send>; delegated Run missing/unknown-verb error includes relay. No top-level help alias.")
    sub("orchestrate spawn", orchestrate, "runSpawn", "orchestrate spawn --role <role> [flags] <prompt>", "Requires >=1 positional, then nonempty role; first positional is prompt, extras ignored. same-tree=true warns before env validation; false is omitted from body.", "orchestrate spawn")
    sub("orchestrate send", orchestrate, "runSend", "orchestrate send --role <role> <text>", "Requires >=1 positional, then nonempty role; first positional is text, extras ignored.", "orchestrate send")
    sub("orchestrate relay", orchestrate, "runRelay", "orchestrate relay --plan <path> [flags]", "Start has no start verb. Empty/flag-first argv enters start parser; unknown non-flag first token errors. Plan is trim+absolute; extra positionals ignored; same-tree warning occurs after env validation.", "orchestrate relay")
    sub("orchestrate relay status", orchestrate, "runRelayStatus", "orchestrate relay status [--id <id>]", "No positionals allowed; trimmed nonempty ID filters results, absent result errors; unfiltered empty result prints no relays.", "orchestrate relay status")
    sub("orchestrate relay stop", orchestrate, "runRelayStop", "orchestrate relay stop [--id <id>]", "No positionals allowed; trimmed ID sent; Hub decides selection when omitted.", "orchestrate relay stop")
    sub("usage-relay claude", relay, "Run", "usage-relay --provider claude [--hub <url> --session <id> --token <deprecated>]", "Provider is a flag, not a positional subcommand; parses stdin JSON and emits a status line. Invalid input/post failures warn and return nil.", "usage-relay")
    sub("usage-relay codex", relay, "Run", "usage-relay --provider codex [--hub <url> --session <id> --token <deprecated>]", "Reads stdin JSON transcript_path then only token metadata from its rollout. No status stdout. Invalid input/read/post failures warn and return nil.", "usage-relay")
    sub("wrap <provider>", wrapper, "Run", "wrap <provider> [wrapper flags] [--] [provider argv]", "Same parser for seven aliases and configured custom IDs; ignores fs.Parse error. Positional or -- ends wrapper parsing, rest goes through provider-specific reconstruction. Unknown leading option is consumed by Go's failed parse; remaining args survive.", "wrap")
    help_success = {"many-ai-cli-launcher", "connect", "profile-export", "setup", "issue"}
    flag_sets = []
    for entry in lexical["flag_sets"]:
        flag_sets.append({**entry,
            "lexical_grammar": "Go flag.ContinueOnError: one/two dashes, =value, bool consumes no next token, stop on first positional or --, repeated flags last value wins, int uses base 0 with platform int width",
            "parse_error_handling": "ignored; wrapper continues with partial flags and fs.Args" if entry["name"] == "wrap" else "returned except ErrHelp" if entry["name"] in help_success else "returned including ErrHelp",
            "help_process_exit": None if entry["name"] == "wrap" else 0 if entry["name"] in help_success else 1,
            "stderr": "Go flag diagnostic/usage; top-level main prints returned error once more unless ErrHelp is swallowed",
            "rust_lexical_status": "covered by launcher oracle tests" if entry["name"] == "many-ai-cli-launcher" else "pending delegated parser integration",
        })
    contracts = [
        contract("main-pre-dispatch", "LoadOrCreate runs before every command including help/version; unknown commands can therefore touch configuration or fail first. No arguments also loads config and can open/start Hub.", main, "func run(args"),
        contract("main-exit", "run error prints error plus newline to stderr and exits 1; nil exits 0. Panic logs to hub.log (stderr fallback), then exit 1. There is no generic child exit-code passthrough.", main, "func main()"),
        contract("main-version", "Trim injected version; nonempty non-dev wins with one v prefix removed; otherwise executable-directory then cwd repository tag discovery; fallback dev. Prints version newline.", main, "func displayVersion()", "cmd/many-ai-cli/main_test.go::TestDisplayVersionTrimsVPrefix"),
        contract("main-usage", "Top-level help aliases ignore trailing args and print one usage line on stdout. Provider, usage-relay and orchestrate are absent from that line, but only the latter two are explicitly hidden in source.", main, "func usage()"),
        contract("ignored-tail", "version/help/status/tray/stop/shell-init ignore trailing argv. Flag-based commands stop at the first positional; they do not use GNU interspersed flags.", main, "switch cmd {"),
        contract("serve-port", "Only a parsed --port > 0 overrides configured Hub.Port; zero/negative keeps config. Go flag.Int accepts base-0 integer syntax, not just decimal u16.", main, "if *port > 0"),
        contract("connect-selection", "--profile or --last is required; help returns nil before profile load. Explicit profile selection wins over --last in SelectProfile.", main, 'case "connect":'),
        contract("export-json", "Only --json output accepted; indented JSON to stdout. --name/--host/--cwd/--hub-port feed ExportOptions.", main, 'case "profile-export":'),
        contract("doctor-output", "--json writes compact JSON plus newline; text prints checks/fixes; report levels do not themselves produce a nonzero exit.", main, 'case "doctor":'),
        contract("log-clean-order", "Exactly one unparsed positional path is required. Despite the error usage wording, -o after input becomes extra positionals and fails. Without -o, output replaces final extension with .txt; output path prints to stdout.", main, 'case "log-clean":'),
        contract("uninstall-input", "Reads one stdin line; trim/lowercase y or yes confirms; other/empty/EOF cancels successfully. Main may stop the Hub before this confirmation. purge also removes binary.", "internal/uninstall/uninstall.go", "func confirm("),
        contract("issue-input", "At most one positional title; nonblank --title conflicts with positional. Provider is trimmed and must be one of seven IDs or empty. Without a title prompt to stderr/read one stdin line; blank symptom errors.", issue, "func runIssue(", "cmd/many-ai-cli/issue_test.go::TestRunIssueRejectsConflictingTitleInputs"),
        contract("issue-confirmation", "Only trim/case-insensitive y confirms; yes does not. Dry-run writes redacted Markdown without confirmation. Non-dry output preview precedes stdin confirmation; cancellation succeeds; gh uses --web and fallback remains explicit.", issue, "func confirmIssue(", "cmd/many-ai-cli/issue_test.go::TestRunIssueUsesGHWebAfterConfirmation"),
        contract("wrapper-parse", "Wrapper deliberately ignores parse errors, including help. No success/exit code can be inferred from lexical acceptance or failure alone.", wrapper, "_ = fs.Parse(args)"),
        contract("wrapper-pty-exit", "PTY child wait error is returned; main maps any error to process exit 1. Actual child code/signal remains in session_end metadata.", wrapper, "waitErr := ps.Wait()"),
        contract("wrapper-headless-exit", "Successful headless runner completion returns nil even when result records a nonzero child exit. Launch/runner errors return errors; result State/ExitCode sent in session_end.", "internal/wrapper/headless_session.go", "result, runErr := headless.Run("),
        contract("wrapper-stdio", "Wrapper PTY output is pumped to logs/Hub websocket and input arrives from Hub messages. It is not an inherited-stdio passthrough; stdin terminal dimensions are queried, diagnostics use stderr.", wrapper, "ptyPump(ps, lf,"),
        contract("wrapper-login", "headless + subscription-login rejects; nonempty prompt-file + login rejects; login replaces provider argv with adapter LoginArgs and bypasses model/permission/settings reconstruction.", wrapper, "if *subscriptionLogin {"),
        contract("usage-relay-errors", "Invalid --provider and flag parse errors propagate to main exit 1. Valid-provider stdin decode/read or post failures are warnings and nil (main exit 0); token prefers nonempty env over deprecated argv.", relay, "func Run(args"),
        contract("orchestrate-auth", "Reads session ID, then Hub port, then token; session must parse as positive decimal int. Port/token need only be nonempty here. Token travels in Authorization header, not argv or URL.", orchestrate, "func hubEnv("),
        contract("orchestrate-response", "Request success requires HTTP 200 and response ok true; error detail preferred to error. POST client timeout reports an uncertain pending request, not a canceled child.", orchestrate, "func postChildAPI(", "internal/orchestrate/orchestrate_test.go::TestPostChildAPISpawnTimeoutPendingMessage"),
        contract("relay-role-grammar", "Split first @ then first /, trim parts; provider required, explicit empty effort rejected, ASCII spaces rejected within parts. Other characters/tabs are not generally validated by this parser; actual providers validated downstream.", orchestrate, "func parseRelayProviderModel(", "internal/orchestrate/orchestrate_test.go::TestParseRelayProviderModelWithEffort"),
        contract("launcher-order", "Configure console and print StartupBanner before parsing. Help exits 0 before profile IO; other parse errors appear in FlagSet stderr and again from main, exit 1. Positional/trailing args ignored. Load+Validate precedes UI/direct choice.", launcher, "func run()"),
        contract("launcher-choice", "--ui true takes precedence; otherwise no profile and !last opens UI. --profile or --last selects direct connection. --last false is parsed true then stops at positional false; --last=false is the false form.", launcher, "if *openUI ||"),
        contract("shell-init", "Prints a conditional shell script; functions activate only when MANY_AI_CLI_AUTO=1. Built-in plus effective custom IDs are used, but shell-invalid IDs are omitted and remain usable via wrap.", "internal/shell/init.go", "func InitScriptForProviders("),
        contract("provider-recover-list", "--list is read-only and prints base even without an override; actual history recovery refuses a healthy HEAD.", provider, "func runProviderRecoverCommand(", "cmd/many-ai-cli/provider_command_test.go::TestProviderCommandRecoverListWithoutOverrideOnlyShowsBase"),
    ]
    precedence = [
        contract("usage-token", "Nonempty MANY_AI_CLI_HUB_TOKEN wins over --token (even whitespace); fallback --token produces a deprecation warning. No token value is collected by inventory.", relay, "token := os.Getenv(hubTokenEnv)"),
        contract("wrapper-hub-port", "Positive strconv.Atoi(MANY_AI_CLI_HUB_PORT) overrides config.Hub.Port; missing/invalid/nonpositive env leaves it unchanged.", wrapper, 'os.Getenv("MANY_AI_CLI_HUB_PORT")'),
        contract("spawn-timeout", "Trim MANY_AI_CLI_SPAWN_TIMEOUT; positive time.ParseDuration value wins, otherwise five minutes.", orchestrate, "func spawnClientTimeout()"),
        contract("issue-auto", "MANY_AI_CLI_AUTO exactly 1 blocks non-dry issue before interaction; --dry-run bypasses that guard.", issue, 'deps.getenv("MANY_AI_CLI_AUTO")', "cmd/many-ai-cli/issue_test.go::TestRunIssueAutoModeRejectsBeforeInteraction"),
        contract("delegation", "MANY_AI_CLI_DELEGATION exactly 1/0 overrides config default; all other values fall back. This controls instruction injection, not orchestration permission.", "internal/wrapper/delegation.go", "func DelegationPromptEnabled(", "internal/wrapper/delegation_test.go::TestDelegationPromptEnabledEnvOverridesConfigDefault"),
        contract("export-host", "Ordered deduplicated candidates: --host, MANY_AI_CLI_PUBLIC_HOST, server IP from SSH_CONNECTION, non-loopback/non-link-local IPv4, hostname; trim space/trailing dot and case-insensitive dedupe.", "internal/launcher/export.go", "func exportHostCandidates("),
        contract("export-user", "USERNAME if nonempty, otherwise USER; no trimming in this choice.", "internal/launcher/export.go", 'user := os.Getenv("USERNAME")'),
        contract("parent-shell", "Trimmed MANY_AI_CLI_PARENT_SHELL override wins; otherwise OS parent process detection, Unix SHELL fallback. Windows uses PSModulePath only in its heuristic.", "internal/wrapper/detect_shell_unix.go", "func DetectShell()"),
        contract("subscription-selection", "MANY_AI_CLI_SUBSCRIPTION_ID is trimmed for registration. Selected profile launch env overrides vendor configuration directory; empty profile leaves vendor env unchanged; missing/disabled selection does not silently fall back.", "internal/subscription/resolve.go", "func Resolve("),
        contract("color-env", "force removes inherited NO_COLOR case-insensitively and appends FORCE_COLOR=3/CLICOLOR_FORCE=1; off removes CLICOLOR_FORCE and appends NO_COLOR=1/FORCE_COLOR=0; inherit retains base. All append TERM=xterm-256color, COLORTERM=truecolor, MANY_AI_CLI=1. Headless forces off.", "internal/wrapper/env_color.go", "func childEnv("),
        contract("wrapper-marker", "MANY_AI_CLI exactly 1 suppresses nested Hub auto-start, even before the ordinary IsRunning path.", wrapper, 'os.Getenv("MANY_AI_CLI")'),
        contract("probe-login-markers", "MANY_AI_CLI_USAGE_PROBE and MANY_AI_CLI_SUBSCRIPTION_LOGIN exactly 1 set registration/reattach markers; they are distinct from parsed --subscription-login execution mode.", wrapper, 'os.Getenv("MANY_AI_CLI_USAGE_PROBE")'),
    ]
    precedence += [
        contract("host-label", "SSH_CONNECTION marks SSH and supplies server IP; otherwise SSH_CLIENT/SSH_TTY mark SSH. Nonempty MANY_AI_CLI_HOST_LABEL overrides display host; absent host falls back to local IP.", "internal/hub/runtime.go", "func hostNetInfo()"),
        contract("env-kind", "Nonblank MANY_AI_CLI_ENV_KIND overrides config env kind, then net hint kind, then net-hint SSH, WSL mode, SSH and local inference. Unknown explicit kind normalizes to local.", "internal/hub/runtime.go", "func resolveEnvMeta("),
        contract("provider-homes", "Session-reported vendor directory wins, then process CODEX_HOME/CLAUDE_CONFIG_DIR, then default home directory for approval/skill paths. Registration also reports GROK_HOME. Codex usage-hook config only tests exact empty env; seed ignores directories inside the subscriptions tree.", "internal/hub/approval_handler.go", "func codexAgentsPath("),
        contract("windows-command", "For unresolved .cmd shims execpath and subscription shellSafeCommand use nonempty COMSPEC then the standard Windows cmd path. Default interactive shell instead prefers pwsh/powershell, then an existing COMSPEC path; Unix prefers existing SHELL, bash, sh, then /bin/sh.", "internal/execpath/execpath_windows.go", "func Resolve("),
        contract("path-resolution", "PATH is used for provider executable resolution with expanded absolute entries. Windows can augment entries from registry and expand variables; PATHEXT is consumed indirectly by exec.LookPath. Wrapper diagnostic reads PATH only to explain failures.", "internal/hub/spawn_command.go", "func lookPathLikeSpawn("),
        contract("nim-key", "Trimmed nonempty NVIDIA_API_KEY wins over the private key file and must validate; invalid env errors rather than falling back. Empty env reads the private file. No credential values are collected by these fixtures.", "internal/nvidianim/secret.go", "func ResolveAPIKey("),
        contract("jev-key", "Trimmed TYPESAFE_API_KEY is required for the JEV endpoint; empty yields service-unavailable and no remote request.", "internal/hub/jev_evaluate.go", 'os.Getenv("TYPESAFE_API_KEY")'),
        contract("opencode-config", "NVIDIA NIM route merges provider configuration into inherited OPENCODE_CONFIG_CONTENT; the resulting config and API key overlay child env. This inventory does not enable the route.", "internal/hub/spawn_handler.go", 'os.Getenv("OPENCODE_CONFIG_CONTENT")'),
        contract("whisper-baked", "Trimmed MANY_AI_CLI_WHISPER_SERVER is accepted only for a recognized executable basename and an existing file; otherwise this shortcut is absent and normal managed/external resolution continues.", "internal/hub/whisper_manage.go", "func bakedWhisperServerPath()"),
        contract("windows-directories", "LOCALAPPDATA provides setup application directory and PNPM fallback; APPDATA provides startup shortcut installation/removal. Empty values trigger caller-specific home fallback or omission, not a global path override.", "internal/setupcmd/setup_windows.go", 'os.Getenv("LOCALAPPDATA")'),
        contract("wsl-markers", "On non-Windows, nonempty WSL_INTEROP or WSL_DISTRO_NAME indicates WSL; MANY_AI_CLI_WSL_LAUNCHER exactly 1 distinguishes Windows-launcher mode. Detection is not authorization to launch WSL or query Windows home.", "internal/wslutil/wslutil_unix.go", "func IsWSL()"),
        contract("hub-display-user", "Hub display-name config wins, then USERNAME, then USER. Mobile SSH hints and profile-export use USERNAME then USER without the display-name preference.", "internal/hub/misc_handlers.go", 'userDisplayName := cfg.UserPrefs.DisplayName'),
    ]
    env_names = {
        "usage-token": ["MANY_AI_CLI_HUB_TOKEN"], "wrapper-hub-port": ["MANY_AI_CLI_HUB_PORT"],
        "spawn-timeout": ["MANY_AI_CLI_SPAWN_TIMEOUT"], "issue-auto": ["MANY_AI_CLI_AUTO"],
        "delegation": ["MANY_AI_CLI_DELEGATION"], "export-host": ["MANY_AI_CLI_PUBLIC_HOST", "SSH_CONNECTION"],
        "export-user": ["USERNAME", "USER"], "parent-shell": ["MANY_AI_CLI_PARENT_SHELL", "SHELL", "PSModulePath"],
        "subscription-selection": ["MANY_AI_CLI_SUBSCRIPTION_ID", "CLAUDE_CONFIG_DIR", "CODEX_HOME", "GROK_HOME", "XDG_DATA_HOME"],
        "color-env": ["NO_COLOR", "FORCE_COLOR", "CLICOLOR_FORCE", "TERM", "COLORTERM", "MANY_AI_CLI"],
        "wrapper-marker": ["MANY_AI_CLI"], "probe-login-markers": ["MANY_AI_CLI_USAGE_PROBE", "MANY_AI_CLI_SUBSCRIPTION_LOGIN"],
        "host-label": ["SSH_CONNECTION", "SSH_CLIENT", "SSH_TTY", "MANY_AI_CLI_HOST_LABEL"],
        "env-kind": ["MANY_AI_CLI_ENV_KIND"], "provider-homes": ["CODEX_HOME", "CLAUDE_CONFIG_DIR", "GROK_HOME"],
        "windows-command": ["COMSPEC", "SHELL"], "path-resolution": ["PATH", "PATHEXT"],
        "nim-key": ["NVIDIA_API_KEY"], "jev-key": ["TYPESAFE_API_KEY"], "opencode-config": ["OPENCODE_CONFIG_CONTENT"],
        "whisper-baked": ["MANY_AI_CLI_WHISPER_SERVER"], "windows-directories": ["LOCALAPPDATA", "APPDATA"],
        "wsl-markers": ["WSL_INTEROP", "WSL_DISTRO_NAME", "MANY_AI_CLI_WSL_LAUNCHER"], "hub-display-user": ["USERNAME", "USER"],
    }
    for rule in precedence:
        rule["names"] = env_names[rule["id"]]
    precedence.append({**contract("orchestrate-session-env", "Session ID must be a positive decimal integer, then nonempty port and token are required, in that order. No CLI flags override these environment values.", orchestrate, "func hubEnv("), "names": ["MANY_AI_CLI_SESSION_ID", "MANY_AI_CLI_HUB_PORT", "MANY_AI_CLI_HUB_TOKEN"]})
    covered = {name for rule in precedence for name in rule["names"]}
    missing = {row["name"] for row in lexical["environment_reads"] if "name" in row} - covered
    if missing:
        raise ValueError(f"environment reads lack precedence/consumer rules: {sorted(missing)}")
    contracts += [
        contract("status-stdio", "Prints stopped and succeeds on connection failure or non-200; 200 prints running plus authenticated local URL and optional stale-binary warning. No stdin.", "internal/hub/lifecycle.go", "func PrintStatus("),
        contract("stop-stdio", "No success stdout or stdin; failures return errors to main stderr/exit 1. Uses runtime effective port then config, private logging, graceful shutdown then force-kill fallback. Trial caller must restrict process ownership.", "internal/hub/lifecycle.go", "func Stop("),
        contract("setup-stdio", "No stdin; stdout lists executable/config and created/failed/note shortcut results plus next steps. Any failed shortcut returns an error after reporting all results.", "internal/setupcmd/setup.go", "func Run()"),
        contract("tray-platform", "Non-Windows tray returns ErrUnsupported; Windows runs native tray callbacks without a terminal interaction contract. Runtime/native acceptance pending.", "internal/tray/tray_other.go", "func run("),
    ]

    # Nonliteral environment reads remain explicit. Do not confuse 0 literal
    # matches with no env contract; source constants and dynamic consumers matter.
    dynamic = [
        contract("windows-expand", "The name argument comes from %NAME% expansion. Case-insensitive local cache wins, then nonempty os.Getenv, then registry value without nested %, then PNPM_HOME alone may derive LOCALAPPDATA/pnpm if it exists. Dynamic keys cannot be exhaustively enumerated.", "internal/hub/path_expand_windows.go", "func resolveWinEnvVar("),
        contract("tailscale-search", "Dynamic env loop reads ProgramFiles, ProgramFiles(x86), and ProgramW6432 for executable candidates.", "internal/hub/tailscale_probe_windows.go", '[]string{"ProgramFiles", "ProgramFiles(x86)", "ProgramW6432"}'),
        contract("subscription-seed", "envVar is a vendor config-directory selector supplied by callers; see default directory read rules.", "internal/subscription/seed.go", "os.Getenv(envVar)"),
    ]
    source_files = set(s["path"] for s in lexical["sources"] + pure["sources"])
    source_files.update(r["source"] for r in lexical["environment_reads"])
    source_files.update(r["evidence"]["source"] for r in contracts + precedence + dynamic)
    sources = [{"source": path, "sha256": hashlib.sha256((ROOT / path).read_bytes()).hexdigest()} for path in sorted(source_files)]
    env_constants = []
    for path in sorted((ROOT / "internal").rglob("*.go")):
        if path.name.endswith("_test.go"):
            continue
        for number, line in enumerate(path.read_text().splitlines(), 1):
            match = re.match(r'\s*(?:const\s+)?([A-Za-z_]\w*)\s*=\s*"([A-Z_][A-Z0-9_]*(?:=[^"\n]*)?)"\s*(?://.*)?$', line)
            if match:
                env_constants.append({"identifier": match[1], "value": match[2], "source": str(path.relative_to(ROOT)), "line": number,
                                      "classification": "candidate environment constant; may be emitted rather than directly read"})
    return {
        "schema_version": 2, "baseline": BASELINE,
        "instruction_supplement": "30c37b7 (instruction-only; Go behavior baseline unchanged)",
        "scope": "K01 both binary CLI source contract; parse coverage and runtime acceptance deliberately independent",
        "binaries": ["many-ai-cli", "many-ai-cli-launcher"], "usage": usage, "builtin_providers": builtins,
        "commands": commands, "subcommands": subcommands, "flag_sets": flag_sets,
        "manual_options": [
            {"name": "--distributed", "command": "provider reset", "kind": "required exact first token", "evidence": evidence(provider, 'args[0] != "--distributed"')},
            {"name": "--expected-revision", "command": "provider reset", "kind": "separate next token; empty accepted; last complete occurrence wins", "evidence": evidence(provider, "hasExpected := false")},
            {"name": "--expected-revision", "command": "provider backup restore", "kind": "separate next token; empty rejected; last complete occurrence wins", "evidence": evidence(provider, 'case "restore":')},
            {"name": "--list", "command": "provider recover", "kind": "exact second token; no value; later tokens ignored", "evidence": evidence(provider, 'args[1] == "--list"')},
        ],
        "behavior_contracts": contracts, "environment_precedence": precedence,
        "environment_reads": lexical["environment_reads"], "environment_constants": env_constants,
        "dynamic_environment_reads": dynamic,
        "implicit_environment": [{"names": ["HOME", "USERPROFILE"], "consumer": "os.UserHomeDir in config/profile/other production path resolvers", "status": "production semantics retained; trial mode must not fall back or mutate HOME"},
                                 {"names": ["PATH", "PATHEXT"], "consumer": "Go/OS executable resolution; includes indirect exec.LookPath reads", "status": "OS-specific acceptance remains pending"}],
        "entry_sources": sources,
        "fixtures": {"source_oracle": "rust/tests/fixtures/foundation/cli/source-oracle.json", "flag_case_count": len(lexical["flag_cases"]), "dispatch_case_count": len(lexical["dispatch_cases"]),
                     "pure_oracle": "rust/tests/fixtures/foundation/cli/pure-oracle.json", "pure_case_count": sum(len(rows) for rows in pure["cases"].values()),
                     "rust_tests": "rust/tests/cli_contracts.rs"},
        "acceptance": {"parsed_routes": "covered by source-derived main dispatch tests", "standalone_launcher_lexical": "covered by actual Go flag goldens",
                       "delegated_flag_lexical": "Go goldens recorded; Rust downstream caller implementation pending",
                       "provider_boundary": "original Go parse/control flow observed only until error-only synthetic storage boundary",
                       "runtime_integration": "pending", "native_os_acceptance": "pending"},
        "migration_only": {"flags": ["--trial-root", "--trial-port"], "scope": "leading main-binary pairs only, not consumed inside provider argv", "go_baseline": "not present", "status": "trial-path propagation has separate foundation acceptance; standalone launcher wiring pending"},
        "limitations": ["No application binary was executed to obtain these CLI goldens. No Hub, real home, provider, SSH/WSL, config/settings mutation, or external API was used.",
                        "Dispatch goldens are an AST-grounded projection of source switch/call boundaries, not end-to-end run/stdio observations.",
                        "Go FlagSet goldens are lexical observations; help exit and ignored-error behavior comes from caller source contracts, not flag.ErrHelp alone.",
                        "Pure extracted function fixtures replace only environment reads with a synthetic map and provider history with an error-only boundary spy; no persistence success is simulated.",
                        "Getenv/LookupEnv inventory excludes runtime values; dynamic, os.UserHomeDir/exec.LookPath and child-environment emission are listed separately.",
                        "Source/test evidence references identify existing oracle assertions; they do not claim those whole packages or OS boundaries were run."]}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    data = generate()
    output = json.dumps(data, ensure_ascii=False, indent=2) + "\n"
    path = ROOT / "rust/inventory/cli.json"
    if args.check:
        if not path.exists() or path.read_text() != output:
            raise SystemExit("CLI inventory differs; run python3 rust/scripts/inventory-cli.py")
    else:
        path.write_text(output)
    print(f"{len(data['commands'])} command forms; {len(data['subcommands'])} delegated forms; {len(data['flag_sets'])} FlagSets; {len(data['environment_reads'])} environment-read sites")


if __name__ == "__main__":
    main()
