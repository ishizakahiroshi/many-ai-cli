package hub

import (
	"fmt"

	"many-ai-cli/internal/config"
	"many-ai-cli/internal/provider"
)

func buildProviderRegistry(cfg *config.Config, userLayers ...[]provider.Definition) (*provider.Registry, []provider.Diagnostic, error) {
	var user []provider.Definition
	if len(userLayers) > 0 {
		user = userLayers[0]
	}
	return buildProviderRegistryLayers(cfg, user, nil)
}

func buildProviderRegistryLayers(cfg *config.Config, user, overrides []provider.Definition) (*provider.Registry, []provider.Diagnostic, error) {
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
		Embedded:  definitions,
		Legacy:    legacy,
		User:      user,
		Overrides: overrides,
	}, provider.DefaultAdapterCatalog())
	diagnostics = append(diagnostics, embeddedDiagnostics...)
	diagnostics = append(diagnostics, legacyDiagnostics...)
	if registry == nil {
		return nil, diagnostics, fmt.Errorf("provider registry is nil")
	}
	return registry, diagnostics, nil
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
	registry, diagnostics, err := buildProviderRegistryLayers(s.snapshotCfg(), user, overrides)
	if err != nil {
		return diagnostics, err
	}
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
