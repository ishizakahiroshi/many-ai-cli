package provider

import (
	"fmt"
	"regexp"
	"strconv"
	"strings"

	"many-ai-cli/internal/approval"
	"many-ai-cli/internal/proto"
)

// ApprovalDetector handles provider-specific detection characteristics
// such as feedback cards, native shortcuts, and menu styles.
type ApprovalDetector interface {
	IsFeedbackCard(options []proto.ApprovalOption) bool
	IsNativeShortcut(line string, options []proto.ApprovalOption) (bool, string)
	MenuStyle() string
}

// ApprovalSummarizer extracts conservative display facts from an approval
// prompt while ensuring raw secrets and sensitive tokens are not surfaced.
type ApprovalSummarizer interface {
	Summarize(question, context string) proto.ApprovalSummary
	MaskSecrets(text string) string
}

// ApprovalSender handles formatting the input text sent back to the CLI process.
type ApprovalSender interface {
	FormatSend(option proto.ApprovalOption) string
}

// ApprovalAdapter bundles the detection, summarization, sending, and
// identity persistence contracts for a specific provider approval implementation.
type ApprovalAdapter interface {
	Descriptor() AdapterDescriptor
	Detector() ApprovalDetector
	Summarizer() ApprovalSummarizer
	Sender() ApprovalSender
	RememberKey(candidateKey string) string
}

// Common secret-masking regular expressions to ensure adapters never surface
// raw credential values to the Web UI.
var (
	bearerTokenRe      = regexp.MustCompile(`(?i)\b(Bearer\s+)[A-Za-z0-9_\-\.]{8,}`)
	keyValueSecretRe   = regexp.MustCompile(`(?i)\b((?:api[_-]?key|token|secret|password|auth|private[_-]?key)\s*[:=]\s*["']?)([^"'\s,;]{6,})(["']?)`)
	knownTokenPrefixRe = regexp.MustCompile(`\b(?:sk-[A-Za-z0-9_-]{16,}|ghp_[A-Za-z0-9_-]{16,}|xox[baprs]-[A-Za-z0-9_-]{16,})`)
)

func maskSecretsInText(text string) string {
	if text == "" {
		return ""
	}
	text = bearerTokenRe.ReplaceAllString(text, "${1}[REDACTED]")
	text = keyValueSecretRe.ReplaceAllString(text, "${1}[REDACTED]${3}")
	text = knownTokenPrefixRe.ReplaceAllString(text, "[REDACTED_TOKEN]")
	return text
}

// defaultSummarizer is the shared implementation that wraps internal/approval.Summarize
// with secret masking.
type defaultSummarizer struct{}

func (s defaultSummarizer) MaskSecrets(text string) string {
	return maskSecretsInText(text)
}

func (s defaultSummarizer) Summarize(question, context string) proto.ApprovalSummary {
	maskedQuestion := maskSecretsInText(question)
	maskedContext := maskSecretsInText(context)
	summary := approval.Summarize(maskedQuestion, maskedContext)
	// Guarantee raw and command strings are also strictly masked
	summary.Command = maskSecretsInText(summary.Command)
	summary.Raw = maskSecretsInText(summary.Raw)
	for i, p := range summary.Paths {
		summary.Paths[i] = maskSecretsInText(p)
	}
	return summary
}

// defaultSender formats standard option inputs.
type defaultSender struct {
	withNewline bool
}

func (s defaultSender) FormatSend(option proto.ApprovalOption) string {
	if option.SendText != "" {
		if s.withNewline && !strings.HasSuffix(option.SendText, "\n") {
			return option.SendText + "\n"
		}
		return option.SendText
	}
	res := strconv.Itoa(option.Num)
	if s.withNewline {
		res += "\n"
	}
	return res
}

// baseApprovalAdapter implements the standard ApprovalAdapter methods.
type baseApprovalAdapter struct {
	descriptor AdapterDescriptor
	detector   ApprovalDetector
	summarizer ApprovalSummarizer
	sender     ApprovalSender
}

func (a *baseApprovalAdapter) Descriptor() AdapterDescriptor {
	return a.descriptor
}

func (a *baseApprovalAdapter) Detector() ApprovalDetector {
	return a.detector
}

func (a *baseApprovalAdapter) Summarizer() ApprovalSummarizer {
	return a.summarizer
}

func (a *baseApprovalAdapter) Sender() ApprovalSender {
	return a.sender
}

// RememberKey incorporates the provider ID and the adapter version into the
// key to prevent stale approvals from an older adapter version being misapplied.
func (a *baseApprovalAdapter) RememberKey(candidateKey string) string {
	providerID := a.descriptor.Provider
	if providerID == "" {
		providerID = "generic"
	}
	version := a.descriptor.Version
	if version == "" {
		version = "v1"
	}
	return fmt.Sprintf("%s:%s:%s", providerID, version, candidateKey)
}

// --- Provider-specific detector implementations ---

type defaultDetector struct {
	menuStyle string
}

func (d defaultDetector) IsFeedbackCard(options []proto.ApprovalOption) bool {
	return false
}

func (d defaultDetector) IsNativeShortcut(line string, options []proto.ApprovalOption) (bool, string) {
	return false, ""
}

func (d defaultDetector) MenuStyle() string {
	if d.menuStyle != "" {
		return d.menuStyle
	}
	return "numbered"
}

// Claude-specific detector
type claudeDetector struct {
	defaultDetector
}

var claudeFeedbackActionRe = regexp.MustCompile(`(?i)(\d{1,2})\s+to\s+(review|send|dismiss)\b`)

func (d claudeDetector) IsFeedbackCard(options []proto.ApprovalOption) bool {
	if len(options) == 0 {
		return false
	}
	for _, opt := range options {
		if claudeFeedbackActionRe.MatchString(opt.Label) {
			return true
		}
	}
	return false
}

// Codex-specific detector
type codexDetector struct {
	defaultDetector
}

func (d codexDetector) IsNativeShortcut(line string, options []proto.ApprovalOption) (bool, string) {
	hasSendText := false
	for _, opt := range options {
		if opt.SendText != "" {
			hasSendText = true
			break
		}
	}
	if hasSendText {
		lower := strings.ToLower(strings.TrimSpace(line))
		if lower == "p" || strings.HasSuffix(lower, "(p)") {
			return true, "native_codex_shortcut"
		}
	}
	return false, ""
}

// Global fixed catalog registry for approval adapters
var approvalAdapterRegistry = map[string]ApprovalAdapter{}

func registerApprovalAdapter(adapter ApprovalAdapter) {
	approvalAdapterRegistry[adapter.Descriptor().Key] = adapter
}

func init() {
	commonSummarizer := defaultSummarizer{}
	commonSender := defaultSender{withNewline: true}

	// 1. approval:claude-v1
	registerApprovalAdapter(&baseApprovalAdapter{
		descriptor: AdapterDescriptor{Key: "approval:claude-v1", Kind: AdapterApproval, Version: "v1", Provider: "claude"},
		detector:   claudeDetector{defaultDetector: defaultDetector{menuStyle: "numbered"}},
		summarizer: commonSummarizer,
		sender:     commonSender,
	})

	// 2. approval:codex-v1
	registerApprovalAdapter(&baseApprovalAdapter{
		descriptor: AdapterDescriptor{Key: "approval:codex-v1", Kind: AdapterApproval, Version: "v1", Provider: "codex"},
		detector:   codexDetector{defaultDetector: defaultDetector{menuStyle: "numbered"}},
		summarizer: commonSummarizer,
		sender:     commonSender,
	})

	// 3. approval:copilot-v1
	registerApprovalAdapter(&baseApprovalAdapter{
		descriptor: AdapterDescriptor{Key: "approval:copilot-v1", Kind: AdapterApproval, Version: "v1", Provider: "copilot"},
		detector:   defaultDetector{menuStyle: "numbered"},
		summarizer: commonSummarizer,
		sender:     commonSender,
	})

	// 4. approval:cursor-agent-v1
	registerApprovalAdapter(&baseApprovalAdapter{
		descriptor: AdapterDescriptor{Key: "approval:cursor-agent-v1", Kind: AdapterApproval, Version: "v1", Provider: "cursor-agent"},
		detector:   defaultDetector{menuStyle: "numbered"},
		summarizer: commonSummarizer,
		sender:     commonSender,
	})

	// 5. approval:opencode-v1
	registerApprovalAdapter(&baseApprovalAdapter{
		descriptor: AdapterDescriptor{Key: "approval:opencode-v1", Kind: AdapterApproval, Version: "v1", Provider: "opencode"},
		detector:   defaultDetector{menuStyle: "numbered"},
		summarizer: commonSummarizer,
		sender:     commonSender,
	})

	// 6. approval:grok-v1
	registerApprovalAdapter(&baseApprovalAdapter{
		descriptor: AdapterDescriptor{Key: "approval:grok-v1", Kind: AdapterApproval, Version: "v1", Provider: "grok"},
		detector:   defaultDetector{menuStyle: "numbered"},
		summarizer: commonSummarizer,
		sender:     commonSender,
	})

	// 7. approval:command-code-v1
	registerApprovalAdapter(&baseApprovalAdapter{
		descriptor: AdapterDescriptor{Key: "approval:command-code-v1", Kind: AdapterApproval, Version: "v1", Provider: "command-code"},
		detector:   defaultDetector{menuStyle: "numbered"},
		summarizer: commonSummarizer,
		sender:     commonSender,
	})

	// 8. approval:generic-v1
	registerApprovalAdapter(&baseApprovalAdapter{
		descriptor: AdapterDescriptor{Key: "approval:generic-v1", Kind: AdapterApproval, Version: "v1", Provider: "generic"},
		detector:   defaultDetector{menuStyle: "numbered"},
		summarizer: commonSummarizer,
		sender:     commonSender,
	})
}

// LookupApprovalAdapter looks up an ApprovalAdapter by descriptor key.
// Returns the generic adapter if key is empty, or false if an unknown non-empty key is requested.
func LookupApprovalAdapter(key string) (ApprovalAdapter, bool) {
	if key == "" || key == "none" || key == "approval:generic-v1" {
		return approvalAdapterRegistry["approval:generic-v1"], true
	}
	adapter, ok := approvalAdapterRegistry[key]
	return adapter, ok
}
