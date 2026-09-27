package provider

import (
	"path/filepath"
	"strings"
	"time"
)

// SessionHistoryContext carries the session environment facts needed
// by a history adapter to locate transcripts on disk.
type SessionHistoryContext struct {
	Provider       string
	AgentSessionID string
	CWD            string
	HomeDir        string
	ClaudeDir      string
	CodexHome      string
	StartedAt      string
}

// HistoryPathResolver resolves the candidate transcript file path for a session.
type HistoryPathResolver interface {
	ResolvePath(ctx SessionHistoryContext) (string, bool)
}

// HistoryRecordParser describes parser properties for a transcript format.
type HistoryRecordParser interface {
	Format() string
	CanParse() bool
}

// HistoryAdapter bundles transcript location and record parsing contracts
// for a provider's history implementation.
type HistoryAdapter interface {
	Descriptor() AdapterDescriptor
	PathResolver() HistoryPathResolver
	Parser() HistoryRecordParser
}

type baseHistoryAdapter struct {
	descriptor   AdapterDescriptor
	pathResolver HistoryPathResolver
	parser       HistoryRecordParser
}

func (a *baseHistoryAdapter) Descriptor() AdapterDescriptor {
	return a.descriptor
}

func (a *baseHistoryAdapter) PathResolver() HistoryPathResolver {
	return a.pathResolver
}

func (a *baseHistoryAdapter) Parser() HistoryRecordParser {
	return a.parser
}

// --- Path Resolvers ---

type claudePathResolver struct{}

func (r claudePathResolver) ResolvePath(ctx SessionHistoryContext) (string, bool) {
	root := strings.TrimSpace(ctx.ClaudeDir)
	if root == "" {
		if strings.TrimSpace(ctx.HomeDir) == "" {
			return "", false
		}
		root = filepath.Join(ctx.HomeDir, ".claude")
	}
	if ctx.AgentSessionID != "" {
		// Standard Claude transcript path structure: <root>/projects/<project_hash>/<agentSessionId>.jsonl
		// When agent session ID is known, caller can assemble exact path or search.
		return filepath.Join(root, "projects", ctx.AgentSessionID+".jsonl"), true
	}
	return "", false
}

type codexPathResolver struct{}

func (r codexPathResolver) ResolvePath(ctx SessionHistoryContext) (string, bool) {
	root := strings.TrimSpace(ctx.CodexHome)
	if root == "" {
		if strings.TrimSpace(ctx.HomeDir) == "" {
			return "", false
		}
		root = filepath.Join(ctx.HomeDir, ".codex")
	}
	if ctx.AgentSessionID != "" {
		return filepath.Join(root, "sessions", ctx.AgentSessionID+".jsonl"), true
	}
	return "", false
}

type commandCodePathResolver struct{}

func (r commandCodePathResolver) ResolvePath(ctx SessionHistoryContext) (string, bool) {
	if ctx.HomeDir == "" || ctx.AgentSessionID == "" {
		return "", false
	}
	return filepath.Join(ctx.HomeDir, ".commandcode", "sessions", ctx.AgentSessionID+".jsonl"), true
}

type unsupportedPathResolver struct{}

func (r unsupportedPathResolver) ResolvePath(ctx SessionHistoryContext) (string, bool) {
	return "", false
}

// --- Record Parsers ---

type staticRecordParser struct {
	format   string
	canParse bool
}

func (p staticRecordParser) Format() string {
	return p.format
}

func (p staticRecordParser) CanParse() bool {
	return p.canParse
}

// Registry
var historyAdapterRegistry = map[string]HistoryAdapter{}

func registerHistoryAdapter(adapter HistoryAdapter) {
	historyAdapterRegistry[adapter.Descriptor().Key] = adapter
}

func init() {
	// 1. history:claude-v1
	registerHistoryAdapter(&baseHistoryAdapter{
		descriptor:   AdapterDescriptor{Key: "history:claude-v1", Kind: AdapterHistory, Version: "v1", Provider: "claude"},
		pathResolver: claudePathResolver{},
		parser:       staticRecordParser{format: "claude-jsonl", canParse: true},
	})

	// 2. history:codex-v1
	registerHistoryAdapter(&baseHistoryAdapter{
		descriptor:   AdapterDescriptor{Key: "history:codex-v1", Kind: AdapterHistory, Version: "v1", Provider: "codex"},
		pathResolver: codexPathResolver{},
		parser:       staticRecordParser{format: "codex-jsonl", canParse: true},
	})

	// 3. history:cursor-agent-v1
	registerHistoryAdapter(&baseHistoryAdapter{
		descriptor:   AdapterDescriptor{Key: "history:cursor-agent-v1", Kind: AdapterHistory, Version: "v1", Provider: "cursor-agent"},
		pathResolver: unsupportedPathResolver{},
		parser:       staticRecordParser{format: "cursor-agent-jsonl", canParse: false},
	})

	// 4. history:opencode-v1
	registerHistoryAdapter(&baseHistoryAdapter{
		descriptor:   AdapterDescriptor{Key: "history:opencode-v1", Kind: AdapterHistory, Version: "v1", Provider: "opencode"},
		pathResolver: unsupportedPathResolver{},
		parser:       staticRecordParser{format: "opencode-jsonl", canParse: false},
	})

	// 5. history:command-code-v1
	registerHistoryAdapter(&baseHistoryAdapter{
		descriptor:   AdapterDescriptor{Key: "history:command-code-v1", Kind: AdapterHistory, Version: "v1", Provider: "command-code"},
		pathResolver: commandCodePathResolver{},
		parser:       staticRecordParser{format: "command-code-jsonl", canParse: true},
	})
}

// LookupHistoryAdapter returns the registered HistoryAdapter for the given key.
func LookupHistoryAdapter(key string) (HistoryAdapter, bool) {
	if key == "" || key == "none" || key == "unsupported" {
		return nil, false
	}
	adapter, ok := historyAdapterRegistry[key]
	return adapter, ok
}

// ParseStartedAt parses RFC3339 timestamp helper.
func ParseStartedAt(startedAt string) (time.Time, error) {
	return time.Parse(time.RFC3339, startedAt)
}
