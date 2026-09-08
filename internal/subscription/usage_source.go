package subscription

// usage_source.go is the one table that names, per provider, how many-ai-cli
// can learn its remaining quota (親 plan: docs/local/plan_session-handoff-board.md
// 検討シート B3。子 plan: docs/local/plan_session-handoff-board_c6_usage-dispatch.md).
//
// Before this file, "which providers get a local-file usage read" and "which
// providers can appear in the usage panel" were separate hand-written
// switch/slice literals scattered across internal/hub/subscription_usage.go,
// internal/usagelocal, and internal/hub/handoff.go. Adding a provider meant
// finding every one of those. This file is the single place that decision is
// recorded; callers ask UsageSourceFor instead of matching on the provider
// string themselves.
//
// This does NOT add a fourth source kind where many-ai-cli queries a vendor
// CLI for its quota. That door stays shut for the same reason recorded in
// this package's own doc comment (adapter.go): no supported CLI exposes
// remaining quota through a command, so a query-based UsageSourceKind would
// have no provider that could ever satisfy it. The three kinds below are
// exhaustive of what has actually been observed (measured 2026-08-17,
// re-confirmed for Cursor Agent 2026-09-08 — see
// docs/local/reference/reference_usage-sources.md).
type UsageSourceKind string

const (
	// UsageSourcePushed means a vendor CLI (or a hook it runs) sends usage to
	// the Hub itself; the Hub never opens a file to learn it. Claude's
	// statusLine is the only example today.
	UsageSourcePushed UsageSourceKind = "pushed"
	// UsageSourceLocalFile means the vendor CLI leaves usage data in a file
	// under its own config/data directory, and the Hub rereads that file
	// (internal/usagelocal). Codex (rollout JSONL) and Grok (unified.jsonl)
	// are the examples today.
	UsageSourceLocalFile UsageSourceKind = "local-file"
	// UsageSourceNone means many-ai-cli has no way to learn this provider's
	// remaining quota. A provider in this state must not gain an "Unknown"
	// usage-panel column (子 plan 前提: reading that as an invitation to add
	// one has happened before) — it simply stays out of the panel.
	UsageSourceNone UsageSourceKind = "none"
)

// UsageSource is one row of the dispatch table.
type UsageSource struct {
	// Provider is the many-ai-cli provider id ("claude", "codex", ...).
	Provider string
	// Kind is how (if at all) usage data reaches many-ai-cli for Provider.
	Kind UsageSourceKind
	// CanDetectApproachingLimit reports whether a session on this provider
	// can notice its own quota getting close to the limit, and so can be the
	// *source* end of a handoff (親 plan 検討シート B3: 「上限を見た引き継ぎの
	// 促しは残量が読める provider だけ」). This tracks Kind for every
	// provider today (pushed/local-file are both detectable, none is not),
	// but is kept as its own field rather than derived from Kind because a
	// future source kind is not guaranteed to make detection possible.
	CanDetectApproachingLimit bool
	// CanBeHandoffTarget reports whether a session on this provider can be
	// spawned as a handoff *successor*, independent of whether usage can be
	// read for it. Every built-in provider qualifies today — a target only
	// needs to be launchable, not observable — but the column exists
	// separately so a future provider that is wrap-supported yet cannot be
	// spawned as an ordinary session (none exist today) has somewhere to say
	// so.
	CanBeHandoffTarget bool
}

// usageSources is the table. Adding a provider with a local file to read
// means adding a row here (Kind: UsageSourceLocalFile) plus the row's own
// parser in internal/usagelocal — the parsing logic for a new file format
// cannot be table-driven, only the decision to attempt it can.
var usageSources = []UsageSource{
	{Provider: "claude", Kind: UsageSourcePushed, CanDetectApproachingLimit: true, CanBeHandoffTarget: true},
	{Provider: "codex", Kind: UsageSourceLocalFile, CanDetectApproachingLimit: true, CanBeHandoffTarget: true},
	{Provider: "grok", Kind: UsageSourceLocalFile, CanDetectApproachingLimit: true, CanBeHandoffTarget: true},
	// Copilot / Cursor Agent / opencode: no local file and no push channel
	// were found (子 plan 内部 C2 実測、Cursor Agent は 2026-09-08 に
	// docs/local/reference/reference_usage-sources.md へ記録). This is
	// independent of D-05 (見送り台帳): D-05 is about credentials not moving
	// through an env var, not about whether usage can be read.
	{Provider: "copilot", Kind: UsageSourceNone, CanBeHandoffTarget: true},
	{Provider: "cursor-agent", Kind: UsageSourceNone, CanBeHandoffTarget: true},
	{Provider: "opencode", Kind: UsageSourceNone, CanBeHandoffTarget: true},
}

// UsageSourceFor returns provider's row, or a synthesized UsageSourceNone row
// (CanBeHandoffTarget: false) when provider is not in the table. A provider
// missing from the table entirely is treated as "cannot be reasoned about",
// which is stricter than an explicit UsageSourceNone row (a listed provider
// with Kind none can still be a handoff target — see copilot/cursor-agent/
// opencode above).
func UsageSourceFor(provider string) UsageSource {
	for _, row := range usageSources {
		if row.Provider == provider {
			return row
		}
	}
	return UsageSource{Provider: provider, Kind: UsageSourceNone}
}

// LocalFileUsageProviders returns the provider ids whose Kind is
// UsageSourceLocalFile, in table order. internal/hub/subscription_usage.go
// uses this instead of a hand-written provider comparison to decide which
// providers' profiles are worth resolving a directory for at all.
func LocalFileUsageProviders() []string {
	out := make([]string, 0, len(usageSources))
	for _, row := range usageSources {
		if row.Kind == UsageSourceLocalFile {
			out = append(out, row.Provider)
		}
	}
	return out
}

// HandoffTargetProviders returns every table provider that can be a handoff
// successor, excluding exclude (normally the source session's own provider —
// 子 plan (docs/local/plan_session-handoff-board_c5_handoff-md.md) 内部 C2:
// 同一 provider の別 subscription profile を候補に出さない).
func HandoffTargetProviders(exclude string) []string {
	out := make([]string, 0, len(usageSources))
	for _, row := range usageSources {
		if row.CanBeHandoffTarget && row.Provider != exclude {
			out = append(out, row.Provider)
		}
	}
	return out
}
