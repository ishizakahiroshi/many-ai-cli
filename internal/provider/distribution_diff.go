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
	fields := []struct {
		name string
		cur  string
		cand string
		over string
	}{
		{"display_name", current.DisplayName, candidate.DisplayName, override.DisplayName},
		{"launch.executable", launchExecutable(current), launchExecutable(candidate), launchExecutable(override)},
		{"launch.args", encodeJSON(launchArgs(current)), encodeJSON(launchArgs(candidate)), encodeJSON(launchArgs(override))},
		{"adapters", encodeJSON(current.Adapters), encodeJSON(candidate.Adapters), encodeJSON(override.Adapters)},
		{"capabilities", encodeJSON(current.Capabilities), encodeJSON(candidate.Capabilities), encodeJSON(override.Capabilities)},
	}
	var changes []DistributionFieldDiff
	for _, field := range fields {
		if field.cur == field.cand && field.over == "" {
			continue
		}
		if field.cur == field.cand && field.over == field.cur {
			continue
		}
		if field.cur == field.cand && field.over != "" && field.over != field.cur {
			changes = append(changes, DistributionFieldDiff{
				Field:    field.name,
				Current:  field.cur,
				Override: field.over,
				Conflict: false,
			})
			continue
		}
		if field.cur == field.cand {
			continue
		}
		conflict := field.over != "" && field.over != field.cand && field.over != field.cur
		changes = append(changes, DistributionFieldDiff{
			Field:     field.name,
			Current:   field.cur,
			Candidate: field.cand,
			Override:  field.over,
			Conflict:  conflict,
		})
	}
	return changes
}

func launchExecutable(definition Definition) string {
	if definition.Launch == nil {
		return ""
	}
	return definition.Launch.Executable
}

func launchArgs(definition Definition) []string {
	if definition.Launch == nil {
		return nil
	}
	return definition.Launch.Args
}

func encodeJSON(value any) string {
	if value == nil {
		return ""
	}
	raw, err := json.Marshal(value)
	if err != nil || string(raw) == "null" || string(raw) == "{}" || string(raw) == "[]" {
		return ""
	}
	return string(raw)
}
