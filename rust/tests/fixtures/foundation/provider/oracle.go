// Synthetic, offline fixture generator. Run from the repository root:
// go run ./rust/tests/fixtures/foundation/provider/oracle.go
// Oracle: Go baseline 21d0bc7935a2c4696fb89ccff2e324157a528c2d.
// This program calls only encoding/json, YAML decoding, and the pure legacy
// adapter. It never runs provider commands, opens user config, or uses a service.
package main

import (
	"encoding/json"
	"fmt"
	"gopkg.in/yaml.v3"
	"many-ai-cli/internal/config"
	"many-ai-cli/internal/provider"
	"os"
	"path/filepath"
	"reflect"
	"strings"
)

const destination = "rust/tests/fixtures/foundation/provider"

type wireCase struct {
	Name         string `json:"name"`
	Type         string `json:"type"`
	Input        any    `json:"input"`
	Expected     any    `json:"expected"`
	Rejected     bool   `json:"rejected,omitempty"`
	ExpectedJSON string `json:"expected_json"`
}
type legacyCase struct {
	Name        string                 `json:"name"`
	YAML        string                 `json:"yaml"`
	Raw         config.CustomProviders `json:"raw"`
	Definitions []provider.Definition  `json:"definitions"`
	Diagnostics []provider.Diagnostic  `json:"diagnostics"`
}

func fill(value reflect.Value) {
	switch value.Kind() {
	case reflect.String:
		value.SetString("synthetic-日本語-<&>\u2028")
	case reflect.Bool:
		value.SetBool(true)
	case reflect.Int:
		value.SetInt(17)
	case reflect.Pointer:
		value.Set(reflect.New(value.Type().Elem()))
		fill(value.Elem())
	case reflect.Slice:
		value.Set(reflect.MakeSlice(value.Type(), 1, 1))
		fill(value.Index(0))
	case reflect.Map:
		value.Set(reflect.MakeMap(value.Type()))
		key := reflect.New(value.Type().Key()).Elem()
		key.SetString("synthetic")
		item := reflect.New(value.Type().Elem()).Elem()
		fill(item)
		value.SetMapIndex(key, item)
	case reflect.Struct:
		for i := 0; i < value.NumField(); i++ {
			fill(value.Field(i))
		}
	default:
		panic(value.Kind())
	}
}
func write(name string, value any) {
	raw, err := json.MarshalIndent(value, "", "  ")
	if err != nil {
		panic(err)
	}
	raw = append(raw, '\n')
	if err = os.WriteFile(filepath.Join(destination, name), raw, 0600); err != nil {
		panic(err)
	}
}
func main() {
	values := []any{
		provider.SourceRef{}, provider.Definition{}, provider.LaunchDefinition{}, provider.ModelsDefinition{}, provider.ModelDefinition{}, provider.HeadlessDefinition{}, provider.PresentationDefinition{}, provider.UpdateDefinition{}, provider.AdapterRefs{}, provider.Layers{}, provider.AdapterCatalog{}, provider.CapabilitySummary{}, provider.EffectiveDefinition{}, provider.Summary{}, provider.Diagnostic{},
		provider.LaunchRequest{}, provider.ResolvedLaunch{}, provider.AdapterDescriptor{}, provider.DistributionPayload{}, provider.DistributionBundle{}, provider.DistributionStatus{}, provider.DistributionFieldDiff{}, provider.DistributionProviderDiff{}, provider.RevisionRecord{}, provider.QuarantineRecord{},
	}
	var wire []wireCase
	types := map[string]reflect.Type{}
	for _, zero := range values {
		typ := reflect.TypeOf(zero)
		name := typ.Name()
		types[name] = typ
		wire = append(wire, wireCase{Name: name + "/zero", Type: name, Input: zero, Expected: zero})
		filled := reflect.New(typ).Elem()
		fill(filled)
		wire = append(wire, wireCase{Name: name + "/filled", Type: name, Input: filled.Interface(), Expected: filled.Interface()})
	}
	add := func(name, typ, raw string) {
		input := json.RawMessage(raw)
		value := reflect.New(types[typ])
		err := json.Unmarshal(input, value.Interface())
		wire = append(wire, wireCase{Name: name, Type: typ, Input: input, Expected: value.Elem().Interface(), Rejected: err != nil})
	}
	add("definition/value-structs-still-emitted", "Definition", `{}`)
	add("definition/disabled-overlay", "Definition", `{"id":"custom-disabled","enabled":false,"update":{"enabled":false}}`)
	add("definition/explicit-empty-pointers", "Definition", `{"launch":{},"models":{},"presentation":{},"update":{}}`)
	add("definition/all-null", "Definition", `{"schema_version":null,"id":null,"display_name":null,"description":null,"enabled":null,"launch":null,"models":null,"capabilities":null,"adapters":null,"presentation":null,"approval_pattern_source":null,"source":null,"update":null}`)
	add("definition/null-collection-elements", "Definition", `{"launch":{"args":[null,"--fake"],"executable_candidates":[null],"allowed_env":[null]},"models":{"items":[null,{}]},"capabilities":{"launch":null}}`)
	add("definition/unknown-optional-and-old-version", "Definition", `{"schema_version":0,"id":"historical","future_optional":{"ignored":true},"launch":{"executable":"fake-old","future_flag":true},"source":{"origin":"future-origin","future_source":true},"adapters":{"approval":"future:reader-v9"}}`)
	add("definition/null-nested-value-structs", "Definition", `{"adapters":{"launch":null,"approval":null},"source":{"origin":null,"revision":null}}`)
	add("definition/empty-optional-collections", "Definition", `{"capabilities":{},"launch":{"args":[],"allowed_env":[],"effort_levels":[]},"models":{"items":[]}}`)
	add("definition/bad-enabled-type", "Definition", `{"id":"bad","enabled":"false"}`)
	add("definition/bad-args-type", "Definition", `{"launch":{"args":"--fake"}}`)
	add("definition/bad-schema-fraction", "Definition", `{"schema_version":1.5}`)
	add("definition/bad-map-value", "Definition", `{"capabilities":{"launch":"yes"}}`)
	add("effective/embedded-fields-and-capability-collision", "EffectiveDefinition", `{"id":"fixture","enabled":false,"capabilities":{"headless":true,"future":false},"capabilities_summary":{"launch":true,"headless":false},"field_origins":{"launch.executable":{"origin":"override"},"old":null},"effective_source":{"origin":"legacy"},"revision":"synthetic-r1"}`)
	add("summary/unknown-origin", "Summary", `{"id":"fixture","origin":"future-origin","enabled":false,"capabilities":null,"presentation":{},"icon_image_version":"1700000000"}`)
	add("diagnostic/unknown-severity", "Diagnostic", `{"code":"future","severity":"future-severity","message":"synthetic diagnostic","field":null}`)
	add("layers/nil", "Layers", `{"Embedded":null,"AcceptedDistribution":null,"Distribution":null,"Legacy":null,"User":null,"Overrides":null}`)
	add("layers/empty", "Layers", `{"Embedded":[],"AcceptedDistribution":[],"Distribution":[],"Legacy":[],"User":[],"Overrides":[]}`)
	add("resolved-launch/empty", "ResolvedLaunch", `{"ExecutableCandidates":[],"Args":[],"AllowedEnv":[]}`)
	add("catalog/nil", "AdapterCatalog", `{"Keys":null}`)
	add("catalog/empty", "AdapterCatalog", `{"Keys":{}}`)
	add("catalog/null-map-value", "AdapterCatalog", `{"Keys":{"launch:generic-v1":null}}`)
	add("distribution/empty", "DistributionPayload", `{"definitions":[],"digests":{}}`)
	add("distribution/null-elements", "DistributionPayload", `{"definitions":[null],"digests":{"fixture":null}}`)
	for i := range wire {
		raw, err := json.Marshal(wire[i].Expected)
		if err != nil {
			panic(err)
		}
		wire[i].ExpectedJSON = string(raw)
	}
	write("wire.json", wire)
	fixtures := []struct{ name, yaml string }{
		{"absent", "token: synthetic-stable-token\n"},
		{"wrong-shape", "token: synthetic-stable-token\ncustom_providers: {historical: malformed}\n"},
		{"empty", "token: synthetic-stable-token\ncustom_providers: []\n"},
		{"historical-quotes-and-empty-arguments", `token: synthetic-stable-token
custom_providers:
  - id: ' MY-CLI '
    label: '  My CLI  '
    command: '"C:\Program Files\fake cli\cli.exe" --agent "" --flag="a""b"'
    approval_pattern_source: 'https://example.invalid/synthetic-patterns.json'
    unknown_older_setting: retained-by-go-ignore
    headless: {args: ['--print', '$HOME;literal'], format: ' text ', prompt_via: ' arg '}
  - id: literal-cli
    command: 'fake-cli $HOME %APPDATA% ~ * | && ; > <'
    label: '   '
    headless: {format: text}
  - id: quote-only
    command: '""'
`},
		{"malformed-command-and-headless", `token: synthetic-stable-token
custom_providers:
  - {id: bad-quote, command: 'fake "unterminated'}
  - {id: bad-newline, command: "fake\nnewline"}
  - {id: bad-tab, command: 'fake "tab	here"'}
  - {id: bad-format, command: 'fake-ok --stay', headless: {args: ['--print'], format: unknown}}
  - {id: bad-prompt, command: 'fake-ok', headless: {format: text, prompt_via: env}}
  - {id: bad-arg, command: 'fake-ok', headless: {args: [''], format: text}}
  - {id: bad-control, command: 'fake-ok', headless: {args: ["--print\n--other"], format: text}}
  - {id: good-after, command: 'fake-ok --last', headless: {format: claude-stream-json, prompt_via: stdin}}
`},
		{"built-in-collisions-duplicates-and-invalid-identifiers", `token: synthetic-stable-token
custom_providers:
  - {id: CLAUDE, command: fake-claude}
  - {id: codex, command: fake-codex}
  - {id: copilot, command: fake-copilot}
  - {id: cursor-agent, command: fake-cursor}
  - {id: opencode, command: fake-opencode}
  - {id: grok, command: fake-grok}
  - {id: command-code, command: fake-command-code}
  - {id: shell, command: fake-shell}
  - {id: ' bad/id ', command: fake-invalid}
  - {id: '-bad', command: fake-invalid}
  - {id: ' CUSTOM ', command: 'fake-first --first'}
  - {id: custom, command: fake-duplicate}
  - {id: late, command: fake-last}
`},
		{"historical-tolerant-yaml", `token: synthetic-stable-token
custom_providers:
  - old-scalar
  - {id: missing-command}
  - {command: fake-missing-id}
  - {id: empty-command, command: '  '}
  - {id: 123, command: 456, label: true}
  - {id: invalid-shape, command: [fake, --arg]}
  - {id: invalid-headless-shape, command: fake, headless: 'old-invalid'}
  - {id: null-headless, command: fake-null, headless: null}
  - {id: old-numeric-headless, command: fake-numeric, headless: {args: [23, true], format: text}}
  - {id: future-fields, command: fake-future, future: {ignored: true}, headless: {args: [], format: text, future_format: 9}}
`},
		{"malformed-first-duplicate-stays-first", `token: synthetic-stable-token
custom_providers:
  - {id: same, command: 'fake "bad'}
  - {id: SAME, command: fake-second}
  - {id: still-good, command: fake-good}
`},
		{"headless-boundaries", "token: synthetic-stable-token\ncustom_providers:\n" +
			"  - {id: long-arg, command: fake, headless: {format: text, args: ['" + strings.Repeat("é", 101) + "']}}\n" +
			"  - {id: max-arg, command: fake, headless: {format: text, args: ['" + strings.Repeat("é", 100) + "']}}\n" +
			"  - {id: many-args, command: fake, headless: {format: text, args: [" + strings.TrimSuffix(strings.Repeat("x,", 33), ",") + "]}}\n"},
	}
	var legacy []legacyCase
	for _, f := range fixtures {
		var cfg config.Config
		if err := yaml.Unmarshal([]byte(f.yaml), &cfg); err != nil {
			panic(fmt.Errorf("%s: %w", f.name, err))
		}
		before, _ := json.Marshal(cfg.CustomProviders)
		defs, diagnostics := config.LegacyProviderDefinitions(cfg.CustomProviders)
		after, _ := json.Marshal(cfg.CustomProviders)
		if string(before) != string(after) {
			panic("adapter mutated fixture")
		}
		legacy = append(legacy, legacyCase{f.name, f.yaml, cfg.CustomProviders, defs, diagnostics})
	}
	write("legacy.json", legacy)
	write("catalog.json", provider.DefaultAdapterCatalog())
	write("metadata.json", map[string]any{"go_baseline": "21d0bc7935a2c4696fb89ccff2e324157a528c2d", "generator": "rust/tests/fixtures/foundation/provider/oracle.go", "wire_cases": len(wire), "legacy_cases": len(legacy), "provider_execution": false, "network": false, "catalog_keys": len(provider.DefaultAdapterCatalog().Keys)})
	fmt.Printf("wrote %d wire cases and %d legacy cases\n", len(wire), len(legacy))
}
