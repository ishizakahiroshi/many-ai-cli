package provider

import (
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
)

type Registry struct {
	definitions map[string]EffectiveDefinition
	order       []string
	summaries   []Summary
	diagnostics []Diagnostic
	revision    string
}

func Build(layers Layers, adapters AdapterCatalog) (*Registry, []Diagnostic) {
	merged, diagnostics := mergeLayers(layers, adapters)
	ids := make([]string, 0, len(merged))
	for id := range merged {
		ids = append(ids, id)
	}
	sortIDs(ids)

	registry := &Registry{
		definitions: make(map[string]EffectiveDefinition, len(ids)),
		order:       append([]string(nil), ids...),
		diagnostics: append([]Diagnostic(nil), diagnostics...),
	}
	for _, id := range ids {
		item := merged[id]
		var definition Definition
		if err := json.Unmarshal(marshalRawObject(item.value), &definition); err != nil {
			registry.diagnostics = append(registry.diagnostics, Diagnostic{Code: "invalid_definition", Severity: SeverityError, Field: id, Message: err.Error()})
			continue
		}
		if definition.Enabled == nil {
			enabled := true
			definition.Enabled = &enabled
		}
		effective := EffectiveDefinition{
			Definition:      definition,
			EffectiveSource: item.source,
			FieldOrigins:    cloneSourceRefs(item.fields),
			Capabilities:    deriveCapabilities(definition),
		}
		registry.definitions[id] = effective
	}
	registry.order = registry.order[:0]
	for _, id := range ids {
		if _, ok := registry.definitions[id]; ok {
			registry.order = append(registry.order, id)
		}
	}
	registry.revision = registryRevision(registry.definitions, registry.order)
	for _, id := range registry.order {
		definition := registry.definitions[id]
		definition.Revision = registry.revision
		registry.definitions[id] = definition
		registry.summaries = append(registry.summaries, Summary{
			ID:           id,
			DisplayName:  definition.DisplayName,
			Enabled:      definition.Enabled == nil || *definition.Enabled,
			Origin:       definition.EffectiveSource.Origin,
			Revision:     registry.revision,
			Capabilities: definition.Capabilities,
		})
	}
	sortDiagnostics(registry.diagnostics)
	return registry, append([]Diagnostic(nil), registry.diagnostics...)
}

func (r *Registry) Lookup(id string) (EffectiveDefinition, bool) {
	if r == nil {
		return EffectiveDefinition{}, false
	}
	definition, ok := r.definitions[id]
	if !ok {
		return EffectiveDefinition{}, false
	}
	return cloneEffectiveDefinition(definition), true
}

func (r *Registry) List() []Summary {
	if r == nil {
		return nil
	}
	out := make([]Summary, len(r.summaries))
	copy(out, r.summaries)
	return out
}

func (r *Registry) Diagnostics() []Diagnostic {
	if r == nil {
		return nil
	}
	return append([]Diagnostic(nil), r.diagnostics...)
}

func (r *Registry) Revision() string {
	if r == nil {
		return ""
	}
	return r.revision
}

func deriveCapabilities(definition Definition) CapabilitySummary {
	capabilities := CapabilitySummary{Launch: definition.Launch != nil}
	if definition.Models != nil {
		capabilities.Models = true
	}
	if definition.Launch != nil {
		capabilities.Effort = len(definition.Launch.EffortArgs) > 0
		capabilities.Headless = definition.Launch.Headless != nil
	}
	capabilities.Approval = adapterAvailable(definition.Adapters.Approval)
	capabilities.Transcript = adapterAvailable(definition.Adapters.Transcript)
	capabilities.Usage = adapterAvailable(definition.Adapters.Usage)
	capabilities.Subscription = adapterAvailable(definition.Adapters.Subscription)
	capabilities.Permissions = adapterAvailable(definition.Adapters.Permissions)
	capabilities.Subagents = adapterAvailable(definition.Adapters.Subagents)
	return capabilities
}

func adapterAvailable(key string) bool { return key != "" && key != "none" && key != "unsupported" }

func registryRevision(definitions map[string]EffectiveDefinition, order []string) string {
	type item struct {
		ID         string     `json:"id"`
		Definition Definition `json:"definition"`
	}
	items := make([]item, 0, len(order))
	for _, id := range order {
		if definition, ok := definitions[id]; ok {
			items = append(items, item{ID: id, Definition: definition.Definition})
		}
	}
	raw, _ := json.Marshal(items)
	sum := sha256.Sum256(raw)
	return hex.EncodeToString(sum[:])
}

func cloneSourceRefs(input map[string]SourceRef) map[string]SourceRef {
	if input == nil {
		return nil
	}
	out := make(map[string]SourceRef, len(input))
	for key, value := range input {
		out[key] = value
	}
	return out
}

func cloneEffectiveDefinition(input EffectiveDefinition) EffectiveDefinition {
	out := input
	out.Definition = cloneDefinition(input.Definition)
	out.FieldOrigins = cloneSourceRefs(input.FieldOrigins)
	return out
}

func cloneDefinition(input Definition) Definition {
	out := input
	if input.Launch != nil {
		launch := *input.Launch
		launch.ExecutableCandidates = append([]string(nil), input.Launch.ExecutableCandidates...)
		launch.Args = append([]string(nil), input.Launch.Args...)
		launch.ModelArgs = append([]string(nil), input.Launch.ModelArgs...)
		launch.EffortArgs = append([]string(nil), input.Launch.EffortArgs...)
		launch.EffortLevels = append([]string(nil), input.Launch.EffortLevels...)
		launch.AllowedEnv = append([]string(nil), input.Launch.AllowedEnv...)
		if input.Launch.Headless != nil {
			headless := *input.Launch.Headless
			headless.Args = append([]string(nil), input.Launch.Headless.Args...)
			launch.Headless = &headless
		}
		out.Launch = &launch
	}
	if input.Models != nil {
		models := *input.Models
		models.Items = append([]ModelDefinition(nil), input.Models.Items...)
		out.Models = &models
	}
	if input.Capabilities != nil {
		out.Capabilities = make(map[string]bool, len(input.Capabilities))
		for key, value := range input.Capabilities {
			out.Capabilities[key] = value
		}
	}
	if input.Presentation != nil {
		presentation := *input.Presentation
		out.Presentation = &presentation
	}
	if input.Update != nil {
		update := *input.Update
		update.VersionArgs = append([]string(nil), input.Update.VersionArgs...)
		update.Args = append([]string(nil), input.Update.Args...)
		if input.Update.Enabled != nil {
			enabled := *input.Update.Enabled
			update.Enabled = &enabled
		}
		out.Update = &update
	}
	return out
}
