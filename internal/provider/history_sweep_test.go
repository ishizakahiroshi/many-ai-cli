package provider

import (
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"sort"
	"strings"
	"testing"
)

type overrideFieldSet struct {
	path  []string
	value any
}

// overrideFieldEdit is one save from the settings form: the fields it sets
// (none for "save without changing anything") and a label for the failure
// message.
type overrideFieldEdit struct {
	sets  []overrideFieldSet
	label string
}

func singleFieldEdit(path []string, value any, label string) overrideFieldEdit {
	return overrideFieldEdit{sets: []overrideFieldSet{{path: path, value: value}}, label: label}
}

func (e overrideFieldEdit) name(providerID string) string {
	if len(e.sets) == 0 {
		return providerID + " " + e.label
	}
	parts := make([]string, 0, len(e.sets))
	for _, set := range e.sets {
		parts = append(parts, strings.Join(set.path, "."))
	}
	return fmt.Sprintf("%s %s=%s", providerID, strings.Join(parts, "+"), e.label)
}

// collectOverrideFieldEdits lists one value-changing edit per scalar/array
// leaf of a provider definition, the way the settings form changes a single
// field. A bool is flipped, so a true in the manifest also yields the
// true→false edit that omitempty drops (update.login_may_be_required).
func collectOverrideFieldEdits(prefix []string, value any, out *[]overrideFieldEdit) {
	switch v := value.(type) {
	case map[string]any:
		for key, child := range v {
			collectOverrideFieldEdits(append(append([]string(nil), prefix...), key), child, out)
		}
	case bool:
		*out = append(*out, singleFieldEdit(prefix, !v, "flip"))
	case string:
		*out = append(*out, singleFieldEdit(prefix, "changed-value", "set"))
	case float64:
		*out = append(*out, singleFieldEdit(prefix, v+1, "inc"))
	case []any:
		if len(v) > 0 {
			if _, isObject := v[0].(map[string]any); isObject {
				return
			}
		}
		*out = append(*out, singleFieldEdit(prefix, []any{"changed"}, "set"))
	}
}

// collectOverrideClearEdits lists one edit per non-empty leaf that empties
// it: "" for a string, [] for an array, 0 for a number. Bools are already
// covered by the flip in collectOverrideFieldEdits.
func collectOverrideClearEdits(prefix []string, value any, out *[]overrideFieldEdit) {
	switch v := value.(type) {
	case map[string]any:
		for key, child := range v {
			collectOverrideClearEdits(append(append([]string(nil), prefix...), key), child, out)
		}
	case string:
		if v != "" {
			*out = append(*out, singleFieldEdit(prefix, "", "clear"))
		}
	case float64:
		if v != 0 {
			*out = append(*out, singleFieldEdit(prefix, float64(0), "clear"))
		}
	case []any:
		if len(v) > 0 {
			*out = append(*out, singleFieldEdit(prefix, []any{}, "clear"))
		}
	}
}

func setOverrideField(root map[string]any, path []string, value any) {
	current := root
	for _, key := range path[:len(path)-1] {
		next, ok := current[key].(map[string]any)
		if !ok {
			next = map[string]any{}
			current[key] = next
		}
		current = next
	}
	current[path[len(path)-1]] = value
}

// jsonPathPresent reports whether path is still in the encoded definition.
func jsonPathPresent(object map[string]any, path []string) bool {
	current := object
	for i, key := range path {
		value, ok := current[key]
		if !ok {
			return false
		}
		if i == len(path)-1 {
			return true
		}
		next, ok := value.(map[string]any)
		if !ok {
			return false
		}
		current = next
	}
	return true
}

// fieldCovered reports whether a rejected field name covers path: the same
// field, or an enclosing object that is missing as a whole.
func fieldCovered(fields []string, path []string) bool {
	target := strings.Join(path, ".")
	for _, field := range fields {
		if field == target || strings.HasPrefix(target, field+".") {
			return true
		}
	}
	return false
}

// definitionJSONWithoutSource encodes a definition for an equality check
// that ignores the source bookkeeping.
func definitionJSONWithoutSource(t *testing.T, definition Definition) string {
	t.Helper()
	definition.Source = SourceRef{}
	raw, err := json.Marshal(definition)
	if err != nil {
		t.Fatal(err)
	}
	return string(raw)
}

// TestSaveEffectiveOverrideNeverSilentlyDropsAnEdit edits every built-in
// manifest one save at a time, the way the settings form does, and checks
// that no save both succeeds and loses the edit. It guards two bugs found on
// 2026-09-23:
//
//   - the stored override is only the delta from the manifest, and
//     readRevision re-validates that delta without the manifest, so rules
//     that need a sibling field (update.enabled needs update.args,
//     launch.headless needs format) rejected legitimate one-field edits with
//     422 and left the revision written but HEAD unmoved;
//   - the delta cannot express an emptied field (omitempty drops it), so
//     emptying a field the manifest fills in reported success while the
//     manifest value came back.
//
// Each edit's expected outcome follows from the edit itself: an edit that is
// invalid as a whole definition must fail; an edit whose field disappears
// from the encoded definition must be refused with ErrOverrideClearsValue
// naming that field, with nothing written; every other edit must save, load
// back, and resolve (manifest + stored override) to exactly the edited
// definition.
func TestSaveEffectiveOverrideNeverSilentlyDropsAnEdit(t *testing.T) {
	definitions, _, err := EmbeddedDefinitions()
	if err != nil {
		t.Fatal(err)
	}
	invalid, refused, saved := 0, 0, 0
	refusedFields := map[string]struct{}{}
	for _, baseline := range definitions {
		baseline.Source = SourceRef{}
		raw, err := json.Marshal(baseline)
		if err != nil {
			t.Fatal(err)
		}
		var baseMap map[string]any
		if err := json.Unmarshal(raw, &baseMap); err != nil {
			t.Fatal(err)
		}
		var edits []overrideFieldEdit
		collectOverrideFieldEdits(nil, baseMap, &edits)
		// update.* keys a manifest may omit but the form can still set.
		edits = append(edits,
			singleFieldEdit([]string{"update", "enabled"}, true, "set-true"),
			singleFieldEdit([]string{"update", "enabled"}, false, "set-false"),
			singleFieldEdit([]string{"update", "args"}, []any{"upgrade"}, "set"),
			singleFieldEdit([]string{"update", "version_args"}, []any{"-v"}, "set"),
			singleFieldEdit([]string{"update", "executable"}, "changed-value", "set"),
			singleFieldEdit([]string{"update", "timeout_seconds"}, float64(60), "set"),
		)
		collectOverrideClearEdits(nil, baseMap, &edits)
		edits = append(edits,
			// What the form sends when the update args box is emptied: it
			// forces the "show the update button" toggle OFF at the same time.
			overrideFieldEdit{sets: []overrideFieldSet{
				{path: []string{"update", "enabled"}, value: false},
				{path: []string{"update", "args"}, value: []any{}},
			}, label: "clear-with-toggle-off"},
			overrideFieldEdit{label: "unchanged"},
		)
	editLoop:
		for _, edit := range edits {
			for _, set := range edit.sets {
				switch set.path[0] {
				case "id", "schema_version", "source":
					continue editLoop
				}
			}
			name := edit.name(baseline.ID)
			desiredMap := map[string]any{}
			if err := json.Unmarshal(raw, &desiredMap); err != nil {
				t.Fatal(err)
			}
			for _, set := range edit.sets {
				setOverrideField(desiredMap, set.path, set.value)
			}
			desiredRaw, err := json.Marshal(desiredMap)
			if err != nil {
				t.Fatal(err)
			}
			var desired Definition
			if err := json.Unmarshal(desiredRaw, &desired); err != nil {
				continue
			}
			normalized, err := json.Marshal(desired)
			if err != nil {
				t.Fatal(err)
			}
			var normalizedMap map[string]any
			if err := json.Unmarshal(normalized, &normalizedMap); err != nil {
				t.Fatal(err)
			}
			var cleared [][]string
			for _, set := range edit.sets {
				if !jsonPathPresent(normalizedMap, set.path) {
					cleared = append(cleared, set.path)
				}
			}
			diagnostics, _ := ValidateDefinition(normalized, DefaultAdapterCatalog())
			store, err := NewHistoryStore(t.TempDir(), t.TempDir())
			if err != nil {
				t.Fatal(err)
			}
			_, saveErr := store.SaveEffectiveOverride(baseline.ID, desired, baseline, "", "edit")
			switch {
			case hasDiagnosticError(diagnostics):
				invalid++
				if saveErr == nil {
					t.Errorf("%s: an invalid definition was saved", name)
				}
			case len(cleared) > 0:
				refused++
				var clearsErr *OverrideClearsValueError
				if !errors.Is(saveErr, ErrOverrideClearsValue) || !errors.As(saveErr, &clearsErr) {
					t.Errorf("%s: emptying a manifest value was not refused: %v", name, saveErr)
					continue
				}
				for _, path := range cleared {
					if !fieldCovered(clearsErr.Fields, path) {
						t.Errorf("%s: refusal fields %v do not name %s", name, clearsErr.Fields, strings.Join(path, "."))
					}
				}
				for _, field := range clearsErr.Fields {
					refusedFields[field] = struct{}{}
				}
				if revisions, err := store.List(baseline.ID); err != nil || len(revisions) != 0 {
					t.Errorf("%s: refused save wrote revisions: %d (err=%v)", name, len(revisions), err)
				}
				if _, err := store.Current(baseline.ID); !os.IsNotExist(err) {
					t.Errorf("%s: refused save left a HEAD: %v", name, err)
				}
			default:
				saved++
				if saveErr != nil {
					t.Errorf("%s: save failed: %v", name, saveErr)
					continue
				}
				overrides, loadDiagnostics, err := store.LoadOverrides()
				if err != nil || len(overrides) != 1 || hasDiagnosticError(loadDiagnostics) {
					t.Errorf("%s: load failed: err=%v overrides=%d diagnostics=%v", name, err, len(overrides), loadDiagnostics)
					continue
				}
				effective, err := mergeDefinitionValues(baseline, overrides[0])
				if err != nil {
					t.Fatal(err)
				}
				if got, want := definitionJSONWithoutSource(t, effective), definitionJSONWithoutSource(t, desired); got != want {
					t.Errorf("%s: saved override resolves to\n%s\nwant\n%s", name, got, want)
				}
			}
		}
	}
	names := make([]string, 0, len(refusedFields))
	for field := range refusedFields {
		names = append(names, field)
	}
	sort.Strings(names)
	t.Logf("invalid=%d refused=%d saved=%d; refused fields: %s", invalid, refused, saved, strings.Join(names, ", "))
	// The kinds of emptied field the 2026-09-23 sweep saw revert silently
	// (bugfix_provider-override-sparse-delta_2026-09-23.md, symptom C2).
	for _, field := range []string{
		"description",
		"adapters.approval", "adapters.permissions", "adapters.subscription", "adapters.transcript", "adapters.usage",
		"launch.headless.prompt_via", "models.source",
		"launch.model_args", "launch.effort_args", "launch.effort_levels", "launch.headless.args", "update.args",
		"update.login_may_be_required",
	} {
		if _, ok := refusedFields[field]; !ok {
			t.Errorf("no sweep edit emptied %s any more; the manifests or the edit generator changed shape", field)
		}
	}
	// 80 refused on 2026-09-25: the 70 of 2026-09-23, plus adapters.subagents
	// on claude/codex/grok (added in b0be5ba) and the toggle-off edit the form
	// sends on all 7 manifests. The 5 that ship update.enabled true cannot
	// empty update.args on its own (invalid); cursor-agent and command-code
	// ship it false (login_may_be_required), so emptying args alone is refused
	// there, as it already was on 09-23.
	if refused < 80 || saved < 100 {
		t.Fatalf("sweep refused %d and saved %d edits; the manifests or the edit generator changed shape", refused, saved)
	}
}
