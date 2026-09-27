package provider

import (
	"encoding/json"
	"fmt"
)

type definitionLayer struct {
	name   string
	origin Origin
	items  []Definition
}

type mergedDefinition struct {
	value  map[string]json.RawMessage
	source SourceRef
	fields map[string]SourceRef
}

func mergeDefinition(base, overlay map[string]json.RawMessage) map[string]json.RawMessage {
	out := cloneRawObject(base)
	for key, value := range overlay {
		if existing, ok := out[key]; ok && isJSONObject(existing) && isJSONObject(value) && key != "source" {
			var left, right map[string]json.RawMessage
			_ = json.Unmarshal(existing, &left)
			_ = json.Unmarshal(value, &right)
			out[key] = marshalRawObject(mergeDefinition(left, right))
			continue
		}
		out[key] = append(json.RawMessage(nil), value...)
	}
	return out
}

func mergeLayers(layers Layers, adapters AdapterCatalog) (map[string]mergedDefinition, []Diagnostic) {
	ordered := []definitionLayer{
		{name: "embedded", origin: OriginEmbedded, items: layers.Embedded},
	}
	distribution := layers.AcceptedDistribution
	if len(distribution) == 0 {
		distribution = layers.Distribution
	}
	ordered = append(ordered,
		definitionLayer{name: "distribution", origin: OriginDistribution, items: distribution},
		definitionLayer{name: "legacy", origin: OriginLegacy, items: layers.Legacy},
		definitionLayer{name: "user", origin: OriginUser, items: layers.User},
		definitionLayer{name: "override", origin: OriginOverride, items: layers.Overrides},
	)

	merged := make(map[string]mergedDefinition)
	var diagnostics []Diagnostic
	for _, layer := range ordered {
		seen := make(map[string]struct{}, len(layer.items))
		for index, definition := range layer.items {
			if definition.ID == "" {
				diagnostics = append(diagnostics, Diagnostic{Code: "missing_id", Severity: SeverityError, Field: fmt.Sprintf("%s[%d].id", layer.name, index), Message: "provider id is required"})
				continue
			}
			if _, exists := seen[definition.ID]; exists {
				diagnostics = append(diagnostics, Diagnostic{Code: "duplicate_id", Severity: SeverityError, Field: fmt.Sprintf("%s[%d].id", layer.name, index), Message: "provider id is duplicated in this layer"})
				continue
			}
			seen[definition.ID] = struct{}{}
			raw, err := json.Marshal(definition)
			if err != nil {
				diagnostics = append(diagnostics, Diagnostic{Code: "marshal_definition", Severity: SeverityError, Field: definition.ID, Message: err.Error()})
				continue
			}
			var object map[string]json.RawMessage
			if err := json.Unmarshal(raw, &object); err != nil {
				diagnostics = append(diagnostics, Diagnostic{Code: "marshal_definition", Severity: SeverityError, Field: definition.ID, Message: err.Error()})
				continue
			}
			object["id"] = json.RawMessage(fmt.Sprintf("%q", definition.ID))
			if previous, exists := merged[definition.ID]; exists {
				if layer.origin == OriginUser && previous.source.Origin == OriginEmbedded {
					diagnostics = append(diagnostics, Diagnostic{Code: "builtin_collision", Severity: SeverityError, Field: definition.ID, Message: "user definition cannot replace an embedded provider"})
					continue
				}
				if layer.origin == OriginLegacy && previous.source.Origin == OriginEmbedded {
					diagnostics = append(diagnostics, Diagnostic{Code: "builtin_collision", Severity: SeverityWarning, Field: definition.ID, Message: "legacy definition is ignored for an embedded provider"})
					continue
				}
				if layer.origin == OriginUser && previous.source.Origin == OriginLegacy {
					diagnostics = append(diagnostics, Diagnostic{Code: "legacy_replaced", Severity: SeverityWarning, Field: definition.ID, Message: "explicit user definition replaces the legacy definition"})
				}
				object = mergeDefinition(previous.value, object)
			}
			source := SourceRef{Origin: layer.origin}
			if definition.Source.Origin != "" {
				source = definition.Source
				source.Origin = layer.origin
			}
			fields := map[string]SourceRef{}
			for field := range object {
				fields[field] = source
			}
			merged[definition.ID] = mergedDefinition{value: object, source: source, fields: fields}
		}
	}
	for id, item := range merged {
		raw := marshalRawObject(item.value)
		definitionDiagnostics, err := ValidateDefinition(raw, adapters)
		if err != nil {
			diagnostics = append(diagnostics, Diagnostic{Code: "invalid_definition", Severity: SeverityError, Field: id, Message: err.Error()})
			delete(merged, id)
			continue
		}
		for i := range definitionDiagnostics {
			definitionDiagnostics[i].Field = id + "." + definitionDiagnostics[i].Field
		}
		diagnostics = append(diagnostics, definitionDiagnostics...)
	}
	return merged, diagnostics
}

func cloneRawObject(input map[string]json.RawMessage) map[string]json.RawMessage {
	out := make(map[string]json.RawMessage, len(input))
	for key, value := range input {
		out[key] = append(json.RawMessage(nil), value...)
	}
	return out
}

func isJSONObject(raw json.RawMessage) bool {
	var object map[string]json.RawMessage
	return len(raw) > 0 && raw[0] == '{' && json.Unmarshal(raw, &object) == nil
}

func marshalRawObject(object map[string]json.RawMessage) json.RawMessage {
	raw, _ := json.Marshal(object)
	return raw
}
