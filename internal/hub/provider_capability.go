package hub

import (
	"many-ai-cli/internal/config"
	"many-ai-cli/internal/provider"
)

func (s *Server) effortWrapArgsForProvider(providerID, effort string) []string {
	if effort == "" {
		return nil
	}
	if definition, ok := s.providerDefinition(providerID); ok {
		args, err := provider.EffortArgs(definition, effort)
		if err == nil && len(args) > 0 {
			return args
		}
		if definition.Launch != nil && len(definition.Launch.EffortArgs) > 0 {
			return nil
		}
	}
	return effortWrapArgs(providerID, effort)
}

func (s *Server) headlessCapableProviderFromRegistry(providerID string, cfg *config.Config) bool {
	if definition, ok := s.providerDefinition(providerID); ok && definition.Launch != nil && definition.Launch.Headless != nil {
		return true
	}
	_, ok := config.HeadlessDefFor(providerID, cfg)
	return ok
}

func (s *Server) effortLevelsByProvider() map[string][]string {
	out := effortLevelsByProvider()
	if registry := s.providerRegistrySnapshot(); registry != nil {
		for _, summary := range registry.List() {
			definition, ok := registry.Lookup(summary.ID)
			if !ok || definition.Launch == nil || len(definition.Launch.EffortLevels) == 0 {
				continue
			}
			out[summary.ID] = append([]string(nil), definition.Launch.EffortLevels...)
		}
	}
	return out
}

func (s *Server) headlessProvidersFromRegistry(cfg *config.Config) []string {
	seen := make(map[string]struct{})
	out := make([]string, 0)
	for _, id := range config.HeadlessProviders(cfg) {
		seen[id] = struct{}{}
		out = append(out, id)
	}
	if registry := s.providerRegistrySnapshot(); registry != nil {
		for _, summary := range registry.List() {
			definition, ok := registry.Lookup(summary.ID)
			if !ok || definition.Launch == nil || definition.Launch.Headless == nil {
				continue
			}
			if _, exists := seen[summary.ID]; exists {
				continue
			}
			seen[summary.ID] = struct{}{}
			out = append(out, summary.ID)
		}
	}
	return out
}
