package provider

// UsageSourceKind represents how (if at all) usage data reaches many-ai-cli.
type UsageSourceKind string

const (
	UsageSourcePushed    UsageSourceKind = "pushed"
	UsageSourceLocalFile UsageSourceKind = "local-file"
	UsageSourceNone      UsageSourceKind = "none"
)

// UsageAdapter provides provider-neutral usage facts and source kinds
// without exposing query-based retrieval (per CLAUDE.md architectural rule:
// "残量ソースは provider 直書きの switch/slice ではなく 1 本の表で持つ。
// 問い合わせて取る経路は作らない（ReadUsage を Adapter に足さない）").
type UsageAdapter interface {
	Descriptor() AdapterDescriptor
	SourceKind() UsageSourceKind
	CanDetectApproachingLimit() bool
	CanBeHandoffTarget() bool
}

// SubscriptionAdapter provides provider-neutral subscription facts.
type SubscriptionAdapter interface {
	Descriptor() AdapterDescriptor
	SupportsMultipleProfiles() bool
}

type baseUsageAdapter struct {
	descriptor                AdapterDescriptor
	sourceKind                UsageSourceKind
	canDetectApproachingLimit bool
	canBeHandoffTarget        bool
}

func (a *baseUsageAdapter) Descriptor() AdapterDescriptor {
	return a.descriptor
}

func (a *baseUsageAdapter) SourceKind() UsageSourceKind {
	return a.sourceKind
}

func (a *baseUsageAdapter) CanDetectApproachingLimit() bool {
	return a.canDetectApproachingLimit
}

func (a *baseUsageAdapter) CanBeHandoffTarget() bool {
	return a.canBeHandoffTarget
}

type baseSubscriptionAdapter struct {
	descriptor               AdapterDescriptor
	supportsMultipleProfiles bool
}

func (a *baseSubscriptionAdapter) Descriptor() AdapterDescriptor {
	return a.descriptor
}

func (a *baseSubscriptionAdapter) SupportsMultipleProfiles() bool {
	return a.supportsMultipleProfiles
}

var (
	usageAdapterRegistry        = map[string]UsageAdapter{}
	subscriptionAdapterRegistry = map[string]SubscriptionAdapter{}
)

func registerUsageAdapter(adapter UsageAdapter) {
	usageAdapterRegistry[adapter.Descriptor().Key] = adapter
}

func registerSubscriptionAdapter(adapter SubscriptionAdapter) {
	subscriptionAdapterRegistry[adapter.Descriptor().Key] = adapter
}

func init() {
	// 1. usage:claude-v1
	registerUsageAdapter(&baseUsageAdapter{
		descriptor:                AdapterDescriptor{Key: "usage:claude-v1", Kind: AdapterUsage, Version: "v1", Provider: "claude"},
		sourceKind:                UsageSourcePushed,
		canDetectApproachingLimit: true,
		canBeHandoffTarget:        true,
	})

	// 2. usage:codex-v1
	registerUsageAdapter(&baseUsageAdapter{
		descriptor:                AdapterDescriptor{Key: "usage:codex-v1", Kind: AdapterUsage, Version: "v1", Provider: "codex"},
		sourceKind:                UsageSourceLocalFile,
		canDetectApproachingLimit: true,
		canBeHandoffTarget:        true,
	})

	// 3. usage:grok-v1
	registerUsageAdapter(&baseUsageAdapter{
		descriptor:                AdapterDescriptor{Key: "usage:grok-v1", Kind: AdapterUsage, Version: "v1", Provider: "grok"},
		sourceKind:                UsageSourceLocalFile,
		canDetectApproachingLimit: true,
		canBeHandoffTarget:        true,
	})

	// 4. usage:opencode-v1
	registerUsageAdapter(&baseUsageAdapter{
		descriptor:                AdapterDescriptor{Key: "usage:opencode-v1", Kind: AdapterUsage, Version: "v1", Provider: "opencode"},
		sourceKind:                UsageSourceNone,
		canDetectApproachingLimit: false,
		canBeHandoffTarget:        true,
	})

	// Subscription adapters
	// 1. subscription:claude-v1
	registerSubscriptionAdapter(&baseSubscriptionAdapter{
		descriptor:               AdapterDescriptor{Key: "subscription:claude-v1", Kind: AdapterSubscription, Version: "v1", Provider: "claude"},
		supportsMultipleProfiles: true,
	})

	// 2. subscription:codex-v1
	registerSubscriptionAdapter(&baseSubscriptionAdapter{
		descriptor:               AdapterDescriptor{Key: "subscription:codex-v1", Kind: AdapterSubscription, Version: "v1", Provider: "codex"},
		supportsMultipleProfiles: true,
	})

	// 3. subscription:grok-v1
	registerSubscriptionAdapter(&baseSubscriptionAdapter{
		descriptor:               AdapterDescriptor{Key: "subscription:grok-v1", Kind: AdapterSubscription, Version: "v1", Provider: "grok"},
		supportsMultipleProfiles: true,
	})

	// 4. subscription:opencode-v1
	registerSubscriptionAdapter(&baseSubscriptionAdapter{
		descriptor:               AdapterDescriptor{Key: "subscription:opencode-v1", Kind: AdapterSubscription, Version: "v1", Provider: "opencode"},
		supportsMultipleProfiles: true,
	})
}

// LookupUsageAdapter returns the registered UsageAdapter for key.
func LookupUsageAdapter(key string) (UsageAdapter, bool) {
	if key == "" || key == "none" || key == "unsupported" {
		return nil, false
	}
	adapter, ok := usageAdapterRegistry[key]
	return adapter, ok
}

// LookupSubscriptionAdapter returns the registered SubscriptionAdapter for key.
func LookupSubscriptionAdapter(key string) (SubscriptionAdapter, bool) {
	if key == "" || key == "none" || key == "unsupported" {
		return nil, false
	}
	adapter, ok := subscriptionAdapterRegistry[key]
	return adapter, ok
}
