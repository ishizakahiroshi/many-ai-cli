package provider

import "sort"

const CurrentSchemaVersion = 1

// BuiltinProviderIDs is the stable order used by the shipped provider
// definitions. Keep shell out: shell is a synthetic launch identity, not an
// AI provider manifest.
var BuiltinProviderIDs = []string{
	"claude",
	"codex",
	"copilot",
	"cursor-agent",
	"opencode",
	"grok",
	"command-code",
}

type Origin string

const (
	OriginEmbedded     Origin = "embedded"
	OriginDistribution Origin = "distribution"
	OriginUser         Origin = "user"
	OriginLegacy       Origin = "legacy"
	OriginOverride     Origin = "override"
)

type SourceRef struct {
	Origin   Origin `json:"origin,omitempty"`
	Version  string `json:"version,omitempty"`
	Digest   string `json:"digest,omitempty"`
	Revision string `json:"revision,omitempty"`
}

type Definition struct {
	SchemaVersion         int                     `json:"schema_version,omitempty"`
	ID                    string                  `json:"id,omitempty"`
	DisplayName           string                  `json:"display_name,omitempty"`
	Description           string                  `json:"description,omitempty"`
	Enabled               *bool                   `json:"enabled,omitempty"`
	Launch                *LaunchDefinition       `json:"launch,omitempty"`
	Models                *ModelsDefinition       `json:"models,omitempty"`
	Capabilities          map[string]bool         `json:"capabilities,omitempty"`
	Adapters              AdapterRefs             `json:"adapters,omitempty"`
	Presentation          *PresentationDefinition `json:"presentation,omitempty"`
	ApprovalPatternSource string                  `json:"approval_pattern_source,omitempty"`
	Source                SourceRef               `json:"source,omitempty"`
	Update                *UpdateDefinition       `json:"update,omitempty"`
}

type LaunchDefinition struct {
	Executable           string              `json:"executable,omitempty"`
	ExecutableCandidates []string            `json:"executable_candidates,omitempty"`
	Args                 []string            `json:"args,omitempty"`
	ModelArgs            []string            `json:"model_args,omitempty"`
	EffortArgs           []string            `json:"effort_args,omitempty"`
	EffortLevels         []string            `json:"effort_levels,omitempty"`
	AllowedEnv           []string            `json:"allowed_env,omitempty"`
	Headless             *HeadlessDefinition `json:"headless,omitempty"`
}

type ModelsDefinition struct {
	AllowCustom bool              `json:"allow_custom,omitempty"`
	Items       []ModelDefinition `json:"items,omitempty"`
	Source      string            `json:"source,omitempty"`
}

type ModelDefinition struct {
	ID    string `json:"id,omitempty"`
	Label string `json:"label,omitempty"`
}

type HeadlessDefinition struct {
	Args      []string `json:"args,omitempty"`
	Format    string   `json:"format,omitempty"`
	PromptVia string   `json:"prompt_via,omitempty"`
}

type PresentationDefinition struct {
	IconText string `json:"icon_text,omitempty"`
	Color    string `json:"color,omitempty"`
}

// UpdateDefinition describes how a provider CLI checks its version and how it
// updates itself. It is read-only data: the argv it produces is resolved by
// the helpers in update.go, never assembled ad hoc by callers, so every
// caller (version check, update job, settings UI) agrees on the same rules.
type UpdateDefinition struct {
	VersionArgs        []string `json:"version_args,omitempty"`
	Args               []string `json:"args,omitempty"`
	Executable         string   `json:"executable,omitempty"`
	Enabled            *bool    `json:"enabled,omitempty"`
	LoginMayBeRequired bool     `json:"login_may_be_required,omitempty"`
	TimeoutSeconds     int      `json:"timeout_seconds,omitempty"`
}

type AdapterRefs struct {
	Launch       string `json:"launch,omitempty"`
	Approval     string `json:"approval,omitempty"`
	Transcript   string `json:"transcript,omitempty"`
	Usage        string `json:"usage,omitempty"`
	Subscription string `json:"subscription,omitempty"`
	Permissions  string `json:"permissions,omitempty"`
	// Subagents selects which registered subagent:* reader (see
	// subagent_adapter.go) builds this provider's subagent tree
	// (docs/local/plan_subagent-tree-popup.md 方針 2). A provider whose own
	// subagent records live in the same place and format as an existing
	// reader can just point here — it does not need a reader of its own.
	Subagents string `json:"subagents,omitempty"`
}

type Layers struct {
	Embedded             []Definition
	AcceptedDistribution []Definition
	// Distribution is retained as an explicit alias for callers that build a
	// snapshot before the distribution acceptance service exists. Build treats
	// AcceptedDistribution as authoritative when it is non-empty.
	Distribution []Definition
	Legacy       []Definition
	User         []Definition
	Overrides    []Definition
}

type AdapterCatalog struct {
	Keys map[string]struct{}
}

func NewAdapterCatalog(keys ...string) AdapterCatalog {
	out := AdapterCatalog{Keys: make(map[string]struct{}, len(keys))}
	for _, key := range keys {
		if key != "" {
			out.Keys[key] = struct{}{}
		}
	}
	return out
}

func DefaultAdapterCatalog() AdapterCatalog {
	return NewAdapterCatalog(
		"approval:claude-v1", "approval:codex-v1", "approval:copilot-v1",
		"approval:cursor-agent-v1", "approval:opencode-v1", "approval:grok-v1",
		"approval:command-code-v1", "approval:generic-v1",
		"history:claude-v1", "history:codex-v1", "history:cursor-agent-v1",
		"history:opencode-v1", "history:command-code-v1",
		"usage:claude-v1", "usage:codex-v1", "usage:opencode-v1", "usage:grok-v1",
		"subscription:claude-v1", "subscription:codex-v1", "subscription:opencode-v1", "subscription:grok-v1",
		"launch:generic-v1", "permissions:generic-v1",
		// subagent: readers are additive per provider. Each key here must be
		// registered in subagent_adapter.go
		// (TestAllCatalogSubagentAdaptersAreRegistered) and have a matching
		// reader in internal/hub/subagent_tree.go's key → reader table (親 C3/C4
		// の完了条件).
		"subagent:claude-v1", "subagent:codex-v1", "subagent:grok-v1",
	)
}

func (c AdapterCatalog) Has(key string) bool {
	if key == "" || key == "none" || key == "unsupported" {
		return true
	}
	_, ok := c.Keys[key]
	return ok
}

type CapabilitySummary struct {
	Launch       bool `json:"launch"`
	Models       bool `json:"models"`
	Effort       bool `json:"effort"`
	Headless     bool `json:"headless"`
	Approval     bool `json:"approval"`
	Transcript   bool `json:"transcript"`
	Usage        bool `json:"usage"`
	Subscription bool `json:"subscription"`
	Permissions  bool `json:"permissions"`
	Subagents    bool `json:"subagents"`
}

type EffectiveDefinition struct {
	Definition
	EffectiveSource SourceRef            `json:"effective_source"`
	FieldOrigins    map[string]SourceRef `json:"field_origins,omitempty"`
	Revision        string               `json:"revision"`
	Capabilities    CapabilitySummary    `json:"capabilities_summary"`
}

type Summary struct {
	ID           string            `json:"id"`
	DisplayName  string            `json:"display_name"`
	Enabled      bool              `json:"enabled"`
	Origin       Origin            `json:"origin"`
	Revision     string            `json:"revision"`
	Capabilities CapabilitySummary `json:"capabilities"`
}

type Severity string

const (
	SeverityWarning Severity = "warning"
	SeverityError   Severity = "error"
)

type Diagnostic struct {
	Code     string   `json:"code"`
	Severity Severity `json:"severity"`
	Field    string   `json:"field,omitempty"`
	Message  string   `json:"message"`
}

func (d Diagnostic) IsError() bool { return d.Severity == SeverityError }

func builtinOrder(id string) int {
	for i, builtin := range BuiltinProviderIDs {
		if builtin == id {
			return i
		}
	}
	return len(BuiltinProviderIDs)
}

func sortIDs(ids []string) {
	sort.SliceStable(ids, func(i, j int) bool {
		iOrder, jOrder := builtinOrder(ids[i]), builtinOrder(ids[j])
		if iOrder != jOrder {
			return iOrder < jOrder
		}
		return ids[i] < ids[j]
	})
}
