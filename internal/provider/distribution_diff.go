package provider

import (
	"encoding/json"
	"sort"
)

type DistributionFieldDiff struct {
	Field     string `json:"field"`
	Current   string `json:"current,omitempty"`
	Candidate string `json:"candidate,omitempty"`
	Override  string `json:"override,omitempty"`
	Conflict  bool   `json:"conflict"`
}

type DistributionProviderDiff struct {
	ID      string                  `json:"id"`
	Status  string                  `json:"status"`
	Changes []DistributionFieldDiff `json:"changes,omitempty"`
}

func DiffDistribution(currentBase, candidateBase, overrides []Definition) []DistributionProviderDiff {
	current := indexDefinitions(currentBase)
	candidate := indexDefinitions(candidateBase)
	override := indexDefinitions(overrides)
	ids := make(map[string]struct{})
	for id := range current {
		ids[id] = struct{}{}
	}
	for id := range candidate {
		ids[id] = struct{}{}
	}
	ordered := make([]string, 0, len(ids))
	for id := range ids {
		ordered = append(ordered, id)
	}
	sort.Strings(ordered)
	result := make([]DistributionProviderDiff, 0, len(ordered))
	for _, id := range ordered {
		_, hasCurrent := current[id]
		_, hasCandidate := candidate[id]
		item := DistributionProviderDiff{ID: id, Status: "unchanged"}
		switch {
		case !hasCurrent && hasCandidate:
			item.Status = "added"
			item.Changes = fieldDiffs(Definition{}, candidate[id], override[id])
		case hasCurrent && !hasCandidate:
			item.Status = "removed"
			item.Changes = fieldDiffs(current[id], Definition{}, override[id])
		default:
			item.Changes = fieldDiffs(current[id], candidate[id], override[id])
			if len(item.Changes) > 0 {
				item.Status = "changed"
			}
		}
		result = append(result, item)
	}
	return result
}

func indexDefinitions(definitions []Definition) map[string]Definition {
	out := make(map[string]Definition, len(definitions))
	for _, definition := range definitions {
		if definition.ID == "" {
			continue
		}
		out[definition.ID] = definition
	}
	return out
}

func fieldDiffs(current, candidate, override Definition) []DistributionFieldDiff {
	currentFields := flattenDefinitionForDiff(current)
	candidateFields := flattenDefinitionForDiff(candidate)
	overrideFields := flattenDefinitionForDiff(override)
	fieldNames := make(map[string]struct{}, len(currentFields)+len(candidateFields)+len(overrideFields))
	for name := range currentFields {
		fieldNames[name] = struct{}{}
	}
	for name := range candidateFields {
		fieldNames[name] = struct{}{}
	}
	for name := range overrideFields {
		fieldNames[name] = struct{}{}
	}
	ordered := make([]string, 0, len(fieldNames))
	for name := range fieldNames {
		if name != "id" {
			ordered = append(ordered, name)
		}
	}
	sort.Strings(ordered)
	var changes []DistributionFieldDiff
	for _, name := range ordered {
		cur := currentFields[name]
		cand := candidateFields[name]
		over, hasOverride := overrideFields[name]
		if cur == cand && !hasOverride {
			continue
		}
		if cur == cand && over == cur {
			continue
		}
		if cur == cand && hasOverride {
			changes = append(changes, DistributionFieldDiff{
				Field:    name,
				Current:  cur,
				Override: over,
				Conflict: false,
			})
			continue
		}
		conflict := hasOverride && over != cand && over != cur
		changes = append(changes, DistributionFieldDiff{
			Field:     name,
			Current:   cur,
			Candidate: cand,
			Override:  over,
			Conflict:  conflict,
		})
	}
	return changes
}

func flattenDefinitionForDiff(definition Definition) map[string]string {
	raw, err := json.Marshal(definition)
	if err != nil {
		return nil
	}
	var value map[string]any
	if err := json.Unmarshal(raw, &value); err != nil {
		return nil
	}
	out := make(map[string]string)
	flattenDistributionValue(out, "", value)
	return out
}

func flattenDistributionValue(out map[string]string, prefix string, value any) {
	if object, ok := value.(map[string]any); ok {
		keys := make([]string, 0, len(object))
		for key := range object {
			keys = append(keys, key)
		}
		sort.Strings(keys)
		for _, key := range keys {
			name := key
			if prefix != "" {
				name = prefix + "." + key
			}
			flattenDistributionValue(out, name, object[key])
		}
		return
	}
	if prefix == "" {
		return
	}
	if text, ok := value.(string); ok {
		out[prefix] = text
		return
	}
	raw, err := json.Marshal(value)
	if err == nil {
		out[prefix] = string(raw)
	}
}
