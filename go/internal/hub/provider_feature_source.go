package hub

// provider_feature_source.go is the one table that says, per provider, where a
// cross-provider feature gets its data (親 plan:
// docs/local/plan_cross-provider-agent-ux-adoption.md C1).
//
// Before this file, two different questions were answered by the same
// hand-written provider list:
//
//   - "can the Hub parse this provider's own transcript into agentChatMessage?"
//     (the chat pane, agent_chat_handler.go)
//   - "is the approval marker block read from that transcript instead of the VT
//     mirror?" (approval_marker_transcript.go)
//
// isAgentChatProvider answered both. That coupling is a trap: the moment a
// provider's transcript becomes readable for the chat pane, its approval marker
// supply would silently move from the VT mirror to the transcript, and approval
// identity (candidateKey + sourceEpoch — approval_identity.go's one-source rule)
// is exactly what must not move by accident. The two columns below are separate
// on purpose, and TestApprovalMarkerSourceDoesNotFollowTranscriptReadability
// fixes that independence.
//
// This table does NOT duplicate the usage table, and must not grow a usage
// column. Where remaining quota can be read from is recorded once, in
// internal/subscription/usage_source.go (including why there is no "ask the
// CLI" source kind); subscription_usage.go asks that table directly.
//
// A provider's manifest (internal/provider/manifests/*.json) may declare an
// adapter — command-code.json declares history:command-code-v1 — while the Hub
// has no parser for it. The manifest declares intent; this table records what
// is implemented. When the two disagree, this table is the one the Hub obeys.
type featureSourceKind string

const (
	// featureSourceNone means the feature has no data source for this provider.
	// A provider in this state must not gain an "unknown" row or column in the
	// UI — it simply stays out (the same discipline as UsageSourceNone).
	featureSourceNone featureSourceKind = "none"
	// featureSourceNative means the data comes from the provider's own
	// structured output (its transcript file).
	featureSourceNative featureSourceKind = "native"
	// featureSourceDerived means the data is derived by many-ai-cli from
	// something that is not the provider's own structured output — today the VT
	// mirror of the terminal.
	featureSourceDerived featureSourceKind = "derived"
)

// providerFeatureSource is one row of the table.
type providerFeatureSource struct {
	// Provider is the many-ai-cli provider id ("claude", "codex", ...).
	Provider string
	// StructuredTranscript reports whether the Hub can read this provider's own
	// transcript as agentChatMessage records (native) or not at all (none).
	// There is no derived value here: a transcript scraped from the terminal is
	// not a transcript, and the chat pane already has its own fallback for
	// providers without one (chat-history.ts renders the session log instead).
	StructuredTranscript featureSourceKind
	// ApprovalMarker reports where the approval marker block is read from:
	// native = the provider's transcript, derived = the VT mirror. It is never
	// none — every wrapped provider can show an approval prompt on screen.
	//
	// native here requires StructuredTranscript native (there is nothing else
	// to read), and TestApprovalMarkerNativeRequiresStructuredTranscript holds
	// the table to it. The reverse does not hold: a provider whose transcript
	// the chat pane can read may keep its approval supply on the VT mirror,
	// which is the whole point of keeping these two columns apart.
	ApprovalMarker featureSourceKind
}

// providerFeatureSources is the table. Adding a provider means adding a row;
// making a provider's transcript readable means changing its
// StructuredTranscript to native *and nothing else* unless the approval supply
// is being moved deliberately (which needs the reasoning in
// approval_marker_transcript.go's opening comment, not just a table edit).
var providerFeatureSources = []providerFeatureSource{
	{Provider: "claude", StructuredTranscript: featureSourceNative, ApprovalMarker: featureSourceNative},
	{Provider: "codex", StructuredTranscript: featureSourceNative, ApprovalMarker: featureSourceNative},
	// copilot writes a structured events.jsonl (user.message / assistant.message /
	// turn boundaries), so this row may well become native. It is not yet: every
	// session sampled on 2026-09-18 had an empty toolRequests, so whether tool
	// calls and their results are recorded is unverified, and a chat pane that
	// shows the talk but silently drops the work would be worse than the terminal.
	{Provider: "copilot", StructuredTranscript: featureSourceNone, ApprovalMarker: featureSourceDerived},
	// cursor-agent / opencode: their history is SQLite, not JSONL — cursor-agent
	// keeps one store.db per chat (with most of the content still in its -wal
	// file), and opencode keeps every session in one shared database with no way
	// to scope a read to one session. Reading either needs a SQLite dependency
	// this repository does not have, so none here is measured, not assumed
	// (2026-09-18 — docs/local/reference/reference_transcript-sources.md).
	{Provider: "cursor-agent", StructuredTranscript: featureSourceNone, ApprovalMarker: featureSourceDerived},
	{Provider: "opencode", StructuredTranscript: featureSourceNone, ApprovalMarker: featureSourceDerived},
	{Provider: "grok", StructuredTranscript: featureSourceNone, ApprovalMarker: featureSourceDerived},
	// command-code: its transcript is Anthropic-shaped and readable (measured
	// 2026-09-18 — docs/local/reference/reference_transcript-sources.md), so the
	// chat pane reads it. **The approval supply stays on the VT mirror**: the
	// confirmation screens (Tool Permission, folder trust) are TUI only and never
	// reach that file, which is why plan_command-code-approval-detector.md built
	// the detector on VT patterns. This row is the reason the two columns exist.
	{Provider: "command-code", StructuredTranscript: featureSourceNative, ApprovalMarker: featureSourceDerived},
}

// providerFeatureSourceFor returns provider's row, or the safe default for a
// provider that is not in the table (a custom provider from config.yaml's
// custom_providers, or one added through the provider registry): no structured
// transcript, approval markers from the VT mirror. That is what an unknown CLI
// gets today, and it is also the conservative answer — a provider the Hub knows
// nothing about must not be assumed to have a readable transcript.
func providerFeatureSourceFor(provider string) providerFeatureSource {
	for _, row := range providerFeatureSources {
		if row.Provider == provider {
			return row
		}
	}
	return providerFeatureSource{
		Provider:             provider,
		StructuredTranscript: featureSourceNone,
		ApprovalMarker:       featureSourceDerived,
	}
}

// providerHasStructuredTranscript reports whether the Hub can read provider's
// own transcript as agentChatMessage records. This is the chat pane's question
// (handleAgentChat, startAgentChatTail, the wrapper_loop resume) and nothing
// else: it must not be used to decide where approvals come from.
func providerHasStructuredTranscript(provider string) bool {
	return providerFeatureSourceFor(provider).StructuredTranscript == featureSourceNative
}

// providerApprovalMarkerFromTranscript reports whether provider's approval
// marker blocks are supplied by its transcript rather than by the VT mirror.
// Callers still have to check that the transcript is actually being read right
// now — approvalMarkerSourceIsTranscriptLocked does that, and is the function
// the rest of the Hub should ask.
func providerApprovalMarkerFromTranscript(provider string) bool {
	return providerFeatureSourceFor(provider).ApprovalMarker == featureSourceNative
}
