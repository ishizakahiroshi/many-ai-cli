package hub

import (
	"errors"
	"fmt"

	"many-ai-cli/internal/config"
	"many-ai-cli/internal/provider"
)

func buildProviderRegistry(cfg *config.Config, userLayers ...[]provider.Definition) (*provider.Registry, []provider.Diagnostic, error) {
	var user []provider.Definition
	if len(userLayers) > 0 {
		user = userLayers[0]
	}
	return buildProviderRegistryLayers(cfg, user, nil, nil)
}

func buildProviderRegistryLayers(cfg *config.Config, user, overrides, accepted []provider.Definition) (*provider.Registry, []provider.Diagnostic, error) {
	definitions, embeddedDiagnostics, err := provider.EmbeddedDefinitions()
	if err != nil {
		return nil, embeddedDiagnostics, err
	}
	var raw config.CustomProviders
	if cfg != nil {
		raw = cfg.CustomProviders
	}
	legacy, legacyDiagnostics := config.LegacyProviderDefinitions(raw)
	registry, diagnostics := provider.Build(provider.Layers{
		Embedded:             definitions,
		AcceptedDistribution: accepted,
		Legacy:               legacy,
		User:                 user,
		Overrides:            overrides,
	}, provider.DefaultAdapterCatalog())
	diagnostics = append(diagnostics, embeddedDiagnostics...)
	diagnostics = append(diagnostics, legacyDiagnostics...)
	if registry == nil {
		return nil, diagnostics, fmt.Errorf("provider registry is nil")
	}
	return registry, diagnostics, nil
}

// loadAcceptedDistributionDefinitions loads the currently accepted
// distribution's definitions for use as a registry layer. "Nothing accepted
// yet" (provider.ErrNoAcceptedDistribution) is a normal, expected state and
// yields an empty layer with no diagnostic; any other failure (corrupted
// pointer, tampered bundle, schema mismatch) must not silently degrade to
// "as if nothing were accepted" — it is surfaced as a diagnostic instead, the
// same way a broken override or user definition already is.
func loadAcceptedDistributionDefinitions(store *provider.DistributionStore) ([]provider.Definition, []provider.Diagnostic) {
	if store == nil {
		return nil, nil
	}
	bundle, err := store.LoadAcceptedBundle()
	if err != nil {
		if errors.Is(err, provider.ErrNoAcceptedDistribution) {
			return nil, nil
		}
		return nil, []provider.Diagnostic{{
			Code:     "distribution_accepted_load_failed",
			Severity: provider.SeverityError,
			Field:    "distribution",
			Message:  err.Error(),
		}}
	}
	return bundle.Payload.Definitions, nil
}

// baselineProviderDefinition resolves what id would look like without any
// override — the embedded/distribution/legacy/user layers only — for use as
// HistoryStore.SaveOverride's validation baseline (see the doc comment on
// SaveOverride). It rebuilds a registry with an empty Overrides layer rather
// than reusing s.providerRegistrySnapshot(), because that cached snapshot
// already has the current override (if any) merged in — exactly the layer
// this baseline must exclude, or a sparse override would validate against
// itself instead of against what it is layered on top of.
func (s *Server) baselineProviderDefinition(id string) provider.Definition {
	var user []provider.Definition
	if s.providerStore != nil {
		if loaded, _, err := s.providerStore.Load(); err == nil {
			user = loaded
		}
	}
	accepted, _ := loadAcceptedDistributionDefinitions(s.distributionStore)
	registry, _, err := buildProviderRegistryLayers(s.snapshotCfg(), user, nil, accepted)
	if err != nil || registry == nil {
		return provider.Definition{ID: id}
	}
	if definition, ok := registry.Lookup(id); ok {
		return definition.Definition
	}
	return provider.Definition{ID: id}
}

func (s *Server) providerRegistrySnapshot() *provider.Registry {
	if s == nil {
		return nil
	}
	s.providerRegistryMu.RLock()
	registry := s.providers
	s.providerRegistryMu.RUnlock()
	if registry != nil {
		return registry
	}
	registry, _, err := buildProviderRegistry(s.snapshotCfg())
	if err != nil {
		return nil
	}
	return registry
}

func (s *Server) reloadProviderRegistry() ([]provider.Diagnostic, error) {
	var user []provider.Definition
	if s.providerStore != nil {
		loaded, diagnostics, err := s.providerStore.Load()
		if err != nil {
			return diagnostics, err
		}
		user = loaded
	}
	var overrides []provider.Definition
	if s.historyStore != nil {
		loaded, historyDiagnostics, err := s.historyStore.LoadOverrides()
		if err != nil {
			return historyDiagnostics, err
		}
		overrides = loaded
	}
	accepted, distributionDiagnostics := loadAcceptedDistributionDefinitions(s.distributionStore)
	registry, diagnostics, err := buildProviderRegistryLayers(s.snapshotCfg(), user, overrides, accepted)
	if err != nil {
		return diagnostics, err
	}
	diagnostics = append(diagnostics, distributionDiagnostics...)
	s.providerRegistryMu.Lock()
	s.providers = registry
	s.providerRegistryMu.Unlock()
	return diagnostics, nil
}

func (s *Server) providerDefinition(id string) (provider.EffectiveDefinition, bool) {
	registry := s.providerRegistrySnapshot()
	if registry == nil {
		return provider.EffectiveDefinition{}, false
	}
	return registry.Lookup(id)
}
