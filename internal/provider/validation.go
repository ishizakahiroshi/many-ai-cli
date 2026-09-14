package provider

import (
	"bytes"
	"encoding/json"
	"fmt"
	"io"
	"regexp"
	"sort"
	"strings"
)

const (
	MaxDefinitionBytes = 256 * 1024
	MaxDefinitionItems = 256
	MaxStringLength    = 512
	MaxArgLength       = 1024
)

var providerIDPattern = regexp.MustCompile(`^[a-z0-9][a-z0-9._-]{0,63}$`)
var envNamePattern = regexp.MustCompile(`^[A-Za-z_][A-Za-z0-9_]*$`)

func ValidateDefinition(raw []byte, adapters AdapterCatalog) ([]Diagnostic, error) {
	if len(raw) > MaxDefinitionBytes {
		return nil, fmt.Errorf("provider definition exceeds %d bytes", MaxDefinitionBytes)
	}
	var object map[string]json.RawMessage
	if err := decodeSingleJSON(raw, &object); err != nil {
		return nil, fmt.Errorf("decode provider definition: %w", err)
	}
	var definition Definition
	if err := json.Unmarshal(raw, &definition); err != nil {
		return nil, fmt.Errorf("decode provider definition fields: %w", err)
	}
	diagnostics := validateDefinitionObject(object, definition, adapters)
	sortDiagnostics(diagnostics)
	return diagnostics, nil
}

func decodeSingleJSON(raw []byte, target any) error {
	decoder := json.NewDecoder(bytes.NewReader(raw))
	decoder.UseNumber()
	if err := decoder.Decode(target); err != nil {
		return err
	}
	var extra any
	if err := decoder.Decode(&extra); err != io.EOF {
		if err == nil {
			return fmt.Errorf("multiple JSON values")
		}
		return err
	}
	return nil
}

func validateDefinitionObject(object map[string]json.RawMessage, definition Definition, adapters AdapterCatalog) []Diagnostic {
	var diagnostics []Diagnostic
	known := map[string]struct{}{
		"schema_version": {}, "id": {}, "display_name": {}, "description": {},
		"enabled": {}, "launch": {}, "models": {}, "capabilities": {},
		"adapters": {}, "presentation": {}, "approval_pattern_source": {}, "source": {},
	}
	for field := range object {
		if _, ok := known[field]; !ok {
			diagnostics = append(diagnostics, Diagnostic{Code: "unknown_field", Severity: SeverityWarning, Field: field, Message: "unknown field is ignored"})
		}
	}
	if definition.SchemaVersion != CurrentSchemaVersion {
		diagnostics = append(diagnostics, Diagnostic{Code: "unsupported_schema_version", Severity: SeverityError, Field: "schema_version", Message: fmt.Sprintf("schema version must be %d", CurrentSchemaVersion)})
	}
	if err := validateID(definition.ID); err != nil {
		diagnostics = append(diagnostics, Diagnostic{Code: "invalid_id", Severity: SeverityError, Field: "id", Message: err.Error()})
	}
	if err := validateText(definition.DisplayName, "display_name", true); err != nil {
		diagnostics = append(diagnostics, Diagnostic{Code: "invalid_display_name", Severity: SeverityError, Field: "display_name", Message: err.Error()})
	}
	if len(definition.Description) > MaxStringLength || containsControl(definition.Description) {
		diagnostics = append(diagnostics, Diagnostic{Code: "invalid_description", Severity: SeverityError, Field: "description", Message: "description is too long or contains a control character"})
	}
	if len(definition.ApprovalPatternSource) > MaxStringLength || containsControl(definition.ApprovalPatternSource) {
		diagnostics = append(diagnostics, Diagnostic{Code: "invalid_approval_pattern_source", Severity: SeverityError, Field: "approval_pattern_source", Message: "approval pattern source is too long or contains a control character"})
	}
	if definition.Launch == nil {
		diagnostics = append(diagnostics, Diagnostic{Code: "missing_launch", Severity: SeverityError, Field: "launch", Message: "launch is required"})
	} else {
		diagnostics = append(diagnostics, validateLaunch(*definition.Launch)...)
	}
	if definition.Models != nil {
		diagnostics = append(diagnostics, validateModels(*definition.Models)...)
	}
	diagnostics = append(diagnostics, validateAdapters(definition.Adapters, adapters)...)
	for key := range definition.Capabilities {
		if !knownCapability(key) {
			diagnostics = append(diagnostics, Diagnostic{Code: "unknown_capability", Severity: SeverityWarning, Field: "capabilities." + key, Message: "unknown capability is ignored"})
		}
	}
	if definition.Presentation != nil {
		if err := validateText(definition.Presentation.IconText, "presentation.icon_text", false); err != nil {
			diagnostics = append(diagnostics, Diagnostic{Code: "invalid_presentation", Severity: SeverityError, Field: "presentation.icon_text", Message: err.Error()})
		}
		if len(definition.Presentation.Color) > MaxStringLength || containsControl(definition.Presentation.Color) {
			diagnostics = append(diagnostics, Diagnostic{Code: "invalid_presentation", Severity: SeverityError, Field: "presentation.color", Message: "presentation color is invalid"})
		}
	}
	return diagnostics
}

func validateID(id string) error {
	if id == "" {
		return fmt.Errorf("id is required")
	}
	if len(id) > 64 || !providerIDPattern.MatchString(id) {
		return fmt.Errorf("id must be a lowercase slug of at most 64 characters")
	}
	if id == "shell" {
		return fmt.Errorf("shell is a reserved launch identity")
	}
	for _, builtin := range BuiltinProviderIDs {
		if id == builtin {
			return nil
		}
	}
	return nil
}

func validateText(value, field string, required bool) error {
	if required && strings.TrimSpace(value) == "" {
		return fmt.Errorf("%s is required", field)
	}
	if len(value) > MaxStringLength || containsControl(value) {
		return fmt.Errorf("%s is too long or contains a control character", field)
	}
	return nil
}

func validateLaunch(launch LaunchDefinition) []Diagnostic {
	var diagnostics []Diagnostic
	candidates := append([]string{}, launch.ExecutableCandidates...)
	if launch.Executable != "" {
		candidates = append([]string{launch.Executable}, candidates...)
	}
	if len(candidates) == 0 {
		diagnostics = append(diagnostics, Diagnostic{Code: "missing_executable", Severity: SeverityError, Field: "launch.executable", Message: "launch.executable or launch.executable_candidates is required"})
	}
	if len(candidates) > MaxDefinitionItems {
		diagnostics = append(diagnostics, Diagnostic{Code: "too_many_executables", Severity: SeverityError, Field: "launch.executable_candidates", Message: "too many executable candidates"})
	}
	for i, value := range candidates {
		if err := validateArg(value, fmt.Sprintf("launch.executable[%d]", i)); err != nil {
			diagnostics = append(diagnostics, Diagnostic{Code: "invalid_executable", Severity: SeverityError, Field: fmt.Sprintf("launch.executable[%d]", i), Message: err.Error()})
		}
	}
	diagnostics = append(diagnostics, validateArgs("launch.args", launch.Args, false)...)
	diagnostics = append(diagnostics, validateArgs("launch.model_args", launch.ModelArgs, true)...)
	diagnostics = append(diagnostics, validateArgs("launch.effort_args", launch.EffortArgs, true)...)
	if len(launch.EffortLevels) > MaxDefinitionItems {
		diagnostics = append(diagnostics, Diagnostic{Code: "too_many_effort_levels", Severity: SeverityError, Field: "launch.effort_levels", Message: "too many effort levels"})
	}
	for i, level := range launch.EffortLevels {
		if err := validateArg(level, fmt.Sprintf("launch.effort_levels[%d]", i)); err != nil {
			diagnostics = append(diagnostics, Diagnostic{Code: "invalid_effort_level", Severity: SeverityError, Field: fmt.Sprintf("launch.effort_levels[%d]", i), Message: err.Error()})
		}
	}
	if len(launch.AllowedEnv) > MaxDefinitionItems {
		diagnostics = append(diagnostics, Diagnostic{Code: "too_many_env_names", Severity: SeverityError, Field: "launch.allowed_env", Message: "too many environment variable names"})
	}
	for i, name := range launch.AllowedEnv {
		if !envNamePattern.MatchString(name) {
			diagnostics = append(diagnostics, Diagnostic{Code: "invalid_env_name", Severity: SeverityError, Field: fmt.Sprintf("launch.allowed_env[%d]", i), Message: "environment entries must be variable names, not values"})
		}
	}
	if launch.Headless != nil {
		diagnostics = append(diagnostics, validateHeadless(*launch.Headless)...)
	}
	return diagnostics
}

func validateArgs(field string, values []string, allowPlaceholders bool) []Diagnostic {
	if len(values) > MaxDefinitionItems {
		return []Diagnostic{{Code: "too_many_args", Severity: SeverityError, Field: field, Message: "too many argv entries"}}
	}
	var diagnostics []Diagnostic
	for i, value := range values {
		if err := validateArg(value, fmt.Sprintf("%s[%d]", field, i)); err != nil {
			diagnostics = append(diagnostics, Diagnostic{Code: "invalid_arg", Severity: SeverityError, Field: fmt.Sprintf("%s[%d]", field, i), Message: err.Error()})
			continue
		}
		if allowPlaceholders {
			for _, placeholder := range []string{"{model}", "{effort}", "{session_id}", "{prompt}"} {
				value = strings.ReplaceAll(value, placeholder, "")
			}
		}
		if strings.ContainsAny(value, "{}") {
			diagnostics = append(diagnostics, Diagnostic{Code: "unknown_placeholder", Severity: SeverityError, Field: fmt.Sprintf("%s[%d]", field, i), Message: "argv contains an unknown placeholder"})
		}
	}
	return diagnostics
}

func validateArg(value, field string) error {
	if value == "" || len(value) > MaxArgLength || containsControl(value) || strings.IndexByte(value, 0) >= 0 {
		return fmt.Errorf("%s is empty, too long, or contains a control character", field)
	}
	if strings.Contains(value, "$(") || strings.Contains(value, "${") || strings.Contains(value, "&&") || strings.Contains(value, ";") || strings.Contains(value, "`") {
		return fmt.Errorf("%s contains shell expansion syntax", field)
	}
	return nil
}

func validateHeadless(headless HeadlessDefinition) []Diagnostic {
	var diagnostics []Diagnostic
	if headless.Format == "" {
		diagnostics = append(diagnostics, Diagnostic{Code: "missing_headless_format", Severity: SeverityError, Field: "launch.headless.format", Message: "headless format is required"})
	}
	if headless.PromptVia != "" && headless.PromptVia != "stdin" && headless.PromptVia != "arg" {
		diagnostics = append(diagnostics, Diagnostic{Code: "invalid_headless_prompt", Severity: SeverityError, Field: "launch.headless.prompt_via", Message: "prompt_via must be stdin or arg"})
	}
	diagnostics = append(diagnostics, validateArgs("launch.headless.args", headless.Args, false)...)
	return diagnostics
}

func validateModels(models ModelsDefinition) []Diagnostic {
	var diagnostics []Diagnostic
	if len(models.Items) > MaxDefinitionItems {
		diagnostics = append(diagnostics, Diagnostic{Code: "too_many_models", Severity: SeverityError, Field: "models.items", Message: "too many model entries"})
	}
	seen := make(map[string]struct{}, len(models.Items))
	for i, model := range models.Items {
		field := fmt.Sprintf("models.items[%d]", i)
		if err := validateID(model.ID); err != nil {
			diagnostics = append(diagnostics, Diagnostic{Code: "invalid_model_id", Severity: SeverityError, Field: field + ".id", Message: err.Error()})
		}
		if _, ok := seen[model.ID]; ok {
			diagnostics = append(diagnostics, Diagnostic{Code: "duplicate_model_id", Severity: SeverityError, Field: field + ".id", Message: "model id is duplicated"})
		}
		seen[model.ID] = struct{}{}
		if err := validateText(model.Label, field+".label", true); err != nil {
			diagnostics = append(diagnostics, Diagnostic{Code: "invalid_model_label", Severity: SeverityError, Field: field + ".label", Message: err.Error()})
		}
	}
	return diagnostics
}

func validateAdapters(refs AdapterRefs, adapters AdapterCatalog) []Diagnostic {
	values := []struct {
		field string
		key   string
	}{
		{"adapters.launch", refs.Launch}, {"adapters.approval", refs.Approval},
		{"adapters.transcript", refs.Transcript}, {"adapters.usage", refs.Usage},
		{"adapters.subscription", refs.Subscription}, {"adapters.permissions", refs.Permissions},
	}
	var diagnostics []Diagnostic
	for _, value := range values {
		if value.key == "" || adapters.Has(value.key) {
			continue
		}
		diagnostics = append(diagnostics, Diagnostic{Code: "unknown_adapter", Severity: SeverityError, Field: value.field, Message: "adapter key is not registered"})
	}
	return diagnostics
}

func knownCapability(key string) bool {
	switch key {
	case "models", "effort", "headless", "approval", "transcript", "usage", "subscription", "permissions":
		return true
	default:
		return false
	}
}

func containsControl(value string) bool {
	for _, r := range value {
		if r < 0x20 || r == 0x7f {
			return true
		}
	}
	return false
}

func sortDiagnostics(diagnostics []Diagnostic) {
	sort.SliceStable(diagnostics, func(i, j int) bool {
		if diagnostics[i].Field != diagnostics[j].Field {
			return diagnostics[i].Field < diagnostics[j].Field
		}
		if diagnostics[i].Code != diagnostics[j].Code {
			return diagnostics[i].Code < diagnostics[j].Code
		}
		return diagnostics[i].Message < diagnostics[j].Message
	})
}
