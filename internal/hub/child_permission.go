package hub

import (
	"strings"

	"many-ai-cli/internal/config"
)

// child_permission.go is the one table that says, per provider, what a child
// session actually starts with at each of the three permission tiers
// (親 plan: docs/local/plan_derived-session-launch.md D3 / 不変条件 5 / 不変条件 7.
// 子 plan: docs/local/plan_derived-session-launch_c2_permission-tiers.md).
//
//	attended  人が見ている。Hub は何も足さない。承認プロンプトは Hub の承認パネルへ来る
//	bounded   聞かないが何でもは許さない。拒否された呼び出しは子が blocked として返す
//	full      今日の既定。全許可（承認バイパス相当）
//
// It follows internal/config/effort.go and internal/subscription/usage_source.go:
// the decision "what does provider X get" lives in exactly one place, and
// callers ask resolveChildPermission instead of matching on the provider string
// themselves. A new `case "claude":` for permissions must not appear elsewhere
// in internal/hub — adding a provider means adding a row here.
//
// 「子だから全許可」をやめる、が本ファイルの目的。誰が起動して誰が見ているか
// （origin）と実行モードで既定の段を決め、provider ごとに段 2 があるかどうかを
// 表が持つ。段 2 が無い provider（grok / cursor-agent / command-code）で bounded を
// 選んだときは段 3 へ落とし、FallbackFrom を立てて確認ダイアログに明示する
// （黙って全許可へ倒さない＝不変条件 5）。
//
// The headless runner (親 plan C5) resolves its children through this same
// function: a headless child is unattended by construction, so it must never
// pick the attended tier just because a human pressed the button that started
// the run. defaultChildPermissionTier is where that rule is written down.
//
// **親 C5 の実装者へ**: headless の子も applyChildPermission を通すこと。非 PTY の
// runner は承認プロンプトを表示する先を持たないので、権限を別に決めたくなるが、
// 段の解決をもう 1 本作ると「確認ダイアログが見せた段」と「実際に起動した段」が
// ずれる（開示が嘘になる）。必要なのは新しい解決ではなく、この表に headless 固有の
// 値が要るかどうかの判断だけ。
//
// 無人の既定（orchestration.child_permission_default）は今日も full＝今日までの挙動。
// bounded へ切り替えるのは、利用者が段 2 で relay 1 周の完走を確認してからの別の
// 判断（子 plan plan_derived-session-launch_c2_permission-tiers.md C4 /
// 見送り台帳 D-12）。テストは relay_test.go の
// TestRelay_startRelay_childPermissionDefaultBounded と ...UnsetStaysFull が
// 切替前後の両方を固定している。

// childApproval is one resolved tier: the approval settings a child would
// actually start with, plus which tier they came from.
type childApproval struct {
	// PermissionMode / Sandbox / AskForApproval are the values the Hub fills
	// into a spawnChildRequest when the caller left them empty. An empty value
	// means "add nothing", so the child keeps its CLI's own default.
	PermissionMode string
	Sandbox        string
	AskForApproval string
	// AllowedTools is the bounded tier's allowlist for this provider, from
	// config (orchestration.bounded_allowed_tools, with a built-in default).
	// Empty for the other two tiers.
	AllowedTools []string
	// RiskConfirmed reports that the child skips the high-risk confirmation the
	// same settings would trigger on a normal /api/spawn request.
	RiskConfirmed bool
	// Tier is the tier these values came from ("attended" / "bounded" /
	// "full"). It names the row that filled the blanks; a value the caller set
	// explicitly is never overwritten and is therefore not described by Tier.
	Tier string
	// FallbackFrom is the tier that was asked for but does not exist for this
	// provider, so the resolution fell through to Tier. Only "bounded" today.
	// Empty whenever the resolved tier is the one that was asked for.
	FallbackFrom string
}

// childPermissionRow is one provider's three tiers. Bounded is nil for a
// provider whose CLI has no way to run unattended without granting everything
// (grok's --always-approve and cursor-agent's --force are all-or-nothing), which
// is the difference the confirmation dialog has to show.
type childPermissionRow struct {
	Provider string
	Attended childApproval
	Bounded  *childApproval
	Full     childApproval
}

// childPermissionTable is the table. Attended is the zero value for every
// provider today — "attended" means the Hub adds nothing and approvals reach
// the Hub's approval panel — and the field exists so a provider that needs an
// explicit flag to *keep* prompting has somewhere to put it.
//
// Flags per tier, measured against each CLI's --help and reference on
// 2026-09-12 (recorded in the 子 plan's 前提 section):
//
//   - claude:   bounded は --permission-mode dontAsk（プロンプトが要る呼び出しを
//     全部自動拒否。事前許可した tool は動く）+ --allowedTools。
//   - codex:    bounded は --ask-for-approval never + --sandbox workspace-write。
//     full との差は sandbox だけ（full は danger-full-access）。
//   - copilot / opencode: bounded の実フラグは wrapper 側の変換に任せ、ここでは
//     内部マーカー config.PermissionModeBounded を置く（copilot は --allow-tool の
//     列挙 + --no-ask-user、opencode は --auto + opencode.json の deny 規則）。
//   - shell:    承認の概念が無い。3 段すべて「何も足さない」。
var childPermissionTable = []childPermissionRow{
	{
		Provider: "claude",
		// RiskConfirmed is true for bounded as well as full. It is not a
		// statement that dontAsk is high-risk: evaluateClaudeRisk also treats a
		// model change as high-risk, so leaving it false would make a bounded
		// child fail to start ("risk confirmation required") whenever its model
		// differs from the last one used — a refusal that has nothing to do
		// with the permission tier. The child's own disclosure is the spawn
		// confirmation dialog, which shows these flags before anyone approves.
		Bounded: &childApproval{PermissionMode: "dontAsk", RiskConfirmed: true},
		Full:    childApproval{PermissionMode: "bypassPermissions", RiskConfirmed: true},
	},
	{
		Provider: "codex",
		// evaluateCodexRisk counts --ask-for-approval never as high-risk on its
		// own (internal/hub/spawn_risk.go), so the bounded tier must carry
		// RiskConfirmed or the child cannot start at all.
		Bounded: &childApproval{Sandbox: "workspace-write", AskForApproval: "never", RiskConfirmed: true},
		Full:    childApproval{Sandbox: "danger-full-access", AskForApproval: "never", RiskConfirmed: true},
	},
	{
		Provider: "copilot",
		Bounded:  &childApproval{PermissionMode: config.PermissionModeBounded, RiskConfirmed: true},
		Full:     childApproval{PermissionMode: "bypassPermissions", RiskConfirmed: true},
	},
	{
		Provider: "opencode",
		Bounded:  &childApproval{PermissionMode: config.PermissionModeBounded, RiskConfirmed: true},
		Full:     childApproval{PermissionMode: "bypassPermissions", RiskConfirmed: true},
	},
	{
		Provider: "grok",
		Full:     childApproval{PermissionMode: "bypassPermissions", RiskConfirmed: true},
	},
	{
		Provider: "cursor-agent",
		Full:     childApproval{PermissionMode: "bypassPermissions", RiskConfirmed: true},
	},
	{
		Provider: "command-code",
		Full:     childApproval{PermissionMode: "bypassPermissions", RiskConfirmed: true},
	},
	{
		// shell sessions run no AI and have no approval prompts to bypass, so
		// every tier is "add nothing". Bounded is non-nil (rather than nil) on
		// purpose: falling back from bounded to full would raise a "this CLI
		// has no bounded tier, starting with full permissions" notice about a
		// session that was never going to be granted anything.
		Provider: "shell",
		Bounded:  &childApproval{},
	},
}

// childPermissionFallbackRow is used for a provider with no row: a custom
// provider from config.yaml, or a value the confirmation dialog kept as a
// selectable option. It matches what the pre-table default branch did — full
// means bypassPermissions, and there is no bounded tier because many-ai-cli
// does not know an arbitrary CLI's flags (the same reason it withholds --model
// and --effort from custom providers).
var childPermissionFallbackRow = childPermissionRow{
	Full: childApproval{PermissionMode: "bypassPermissions", RiskConfirmed: true},
}

// launchOriginConductor / launchOriginUI are the values of
// spawnChildRequest.Origin. The empty string is the conductor (an AI's
// `orchestrate spawn`) and relay, which is what every caller that predates the
// field sends.
//
// They alias the same two values in internal/config, which is where the
// execution-mode resolution reads them (it has to be a pure function, so it
// cannot import this package). One spelling, two names.
const (
	launchOriginConductor = config.LaunchOriginConductor
	launchOriginUI        = config.LaunchOriginUI
)

func childPermissionRowFor(provider string) childPermissionRow {
	provider = strings.TrimSpace(provider)
	for _, row := range childPermissionTable {
		if row.Provider == provider {
			return row
		}
	}
	return childPermissionFallbackRow
}

// defaultChildPermissionTier decides the tier for a request that named no
// permission_preset.
//
// A child a human started from the screen and is watching gets the attended
// tier: its approvals belong in the Hub's approval panel, exactly like a
// session started from New Session. Everything else is unattended — an AI
// conductor's `orchestrate spawn`, and every relay child — and takes the tier
// from orchestration.child_permission_default (unset = full = today).
//
// A headless launch is unattended even when a human pressed the button, because
// there is no terminal for anyone to answer a prompt in. The execution mode read
// here is the **resolved** one (applyChildExecutionMode runs immediately before
// applyChildPermission), so "auto that stayed interactive" cannot reach this
// branch and take the unattended tier by accident.
func defaultChildPermissionTier(origin, executionMode string, cfg config.OrchestrationConfig) string {
	if strings.TrimSpace(origin) == launchOriginUI && strings.TrimSpace(executionMode) != config.ExecutionModeHeadless {
		return config.PermissionPresetAttended
	}
	return cfg.ChildPermissionDefaultTier()
}

// permissionPresetApproval returns the values for one named tier. It is the
// entry point for a launch that names a preset outright — the confirmation
// dialog's tier select, and /api/spawn's permission_preset — where no default
// resolution is needed.
func permissionPresetApproval(provider, preset string, cfg config.OrchestrationConfig) childApproval {
	row := childPermissionRowFor(provider)
	switch strings.TrimSpace(preset) {
	case config.PermissionPresetAttended:
		out := row.Attended
		out.Tier = config.PermissionPresetAttended
		return out
	case config.PermissionPresetBounded:
		if row.Bounded == nil {
			// 不変条件 5: 段 2 が無い provider は段 3 へ落ちるが、落ちたことを
			// 黙らせない。FallbackFrom が確認ダイアログの注意文になる。
			out := row.Full
			out.Tier = config.PermissionPresetFull
			out.FallbackFrom = config.PermissionPresetBounded
			return out
		}
		out := *row.Bounded
		out.Tier = config.PermissionPresetBounded
		out.AllowedTools = cfg.BoundedAllowedToolsFor(provider)
		return out
	default:
		out := row.Full
		out.Tier = config.PermissionPresetFull
		return out
	}
}

// resolveChildPermission is the single resolution every child launch goes
// through: the explicit preset when the request names one, otherwise the
// default for this origin and execution mode.
//
// orchestration.child_full_bypass: false keeps its old meaning — the Hub fills
// nothing in, and the child runs on whatever the caller named in
// permission_mode / sandbox / ask_for_approval. A preset does not override that
// switch: it is the one setting whose whole purpose is "do not auto-grant", and
// a shorthand that re-grants it would empty it out
// (見送り台帳 D-12 / TestRelay_startRelay_childFullBypassOffLeavesApprovalUnset).
func resolveChildPermission(provider, preset, executionMode, origin string, cfg config.OrchestrationConfig) childApproval {
	if !cfg.ChildFullBypassEnabled() {
		return childApproval{Tier: config.PermissionPresetAttended}
	}
	tier := strings.TrimSpace(preset)
	if tier == "" {
		tier = defaultChildPermissionTier(origin, executionMode, cfg)
	}
	return permissionPresetApproval(provider, tier, cfg)
}

// headlessCapableProvider reports whether provider can be launched
// non-interactively: a built-in definition, or a `headless:` block on this
// provider's custom_providers entry. cfg may be nil (built-ins only).
//
// This is the only question internal/hub asks about headless support. What the
// answer is made of — flags, output format, where the prompt goes — stays in
// internal/config/headless.go (親 plan 不変条件 7).
func headlessCapableProvider(provider string, cfg *config.Config) bool {
	_, ok := config.HeadlessDefFor(provider, cfg)
	return ok
}

// applyChildExecutionMode resolves the launch's execution mode and writes the
// result back into the request. **It must run immediately before
// applyChildPermission**, because the permission tier's default reads the
// execution mode: a headless child is unattended even when a human pressed the
// button, and resolving after the tier was chosen would grant the attended tier
// to a session with no terminal to answer a prompt in.
//
// The resolution itself is config.ResolveExecutionMode (a pure function, table
// tested). This wrapper adds the two things that need the Hub's own state: which
// providers have a headless definition, and the orchestration default for a
// request that named no mode at all.
//
// cfg may be nil, which means "built-in definitions only, no configured
// default" — the shape the disclosure preview uses when it has no snapshot.
//
// An error means the caller explicitly asked for headless on a provider that
// cannot do it. It is returned rather than downgraded (親 plan D2): the caller
// turns it into a 400 so the request fails visibly instead of quietly opening an
// interactive session nobody is watching.
func applyChildExecutionMode(body *spawnChildRequest, cfg *config.Config) error {
	if body == nil {
		return nil
	}
	requested := config.NormalizeExecutionMode(body.ExecutionMode)
	if requested == config.ExecutionModeUnset && cfg != nil {
		requested = cfg.Orchestration.ChildExecutionModeDefault()
	}
	capable := headlessCapableProvider(body.Provider, cfg)
	// unattended is left to the origin here. A relay worker is unattended for a
	// second reason (nobody opened it at all), but relay resolves its own
	// children — see internal/hub/relay.go and 子 plan 内部 C4.
	mode, err := config.ResolveExecutionMode(requested, capable, body.Origin, false)
	if err != nil {
		return err
	}
	body.ExecutionMode = mode
	return nil
}

// applyChildPermission fills the approval fields a caller left empty. A value
// the caller set explicitly always wins, which is what keeps every request that
// names its own permission_mode behaving exactly as it did before the tiers
// existed.
func applyChildPermission(body *spawnChildRequest, cfg config.OrchestrationConfig) {
	if body == nil {
		return
	}
	resolved := resolveChildPermission(body.Provider, body.PermissionPreset, body.ExecutionMode, body.Origin, cfg)
	if body.PermissionMode == "" {
		body.PermissionMode = resolved.PermissionMode
	}
	if body.Sandbox == "" {
		body.Sandbox = resolved.Sandbox
	}
	if body.AskForApproval == "" {
		body.AskForApproval = resolved.AskForApproval
	}
	if len(body.AllowedTools) == 0 {
		body.AllowedTools = resolved.AllowedTools
	}
	if resolved.RiskConfirmed {
		body.RiskConfirmed = true
	}
}

// orchestrationCfgForBypass builds the minimal config the two-argument
// compatibility wrappers (applyChildApprovalDefaults / childApprovalPreview)
// stand on: child_full_bypass as given, everything else at its built-in
// default. Those wrappers exist because the tests that pin today's behaviour
// call them with a bool, and that pinning is the proof that this C changed no
// default (子 plan C1 の完了条件).
func orchestrationCfgForBypass(fullBypass bool) config.OrchestrationConfig {
	return config.OrchestrationConfig{ChildFullBypass: &fullBypass}
}
