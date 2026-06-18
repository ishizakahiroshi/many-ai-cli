package hub

import (
	"strings"
	"testing"
)

func TestParseOpenCodeModelsOutput(t *testing.T) {
	out := strings.Join([]string{
		"opencode/big-pickle",
		"{",
		`  "id": "big-pickle",`,
		`  "providerID": "opencode",`,
		`  "name": "Big Pickle",`,
		`  "status": "active"`,
		"}",
		"opencode/nemotron-3-ultra-free",
		"{",
		`  "id": "nemotron-3-ultra-free",`,
		`  "providerID": "opencode",`,
		`  "name": "Nemotron 3 Ultra Free",`,
		`  "status": "inactive"`,
		"}",
		"opencode/deepseek-v4-flash-free",
		"{",
		`  "id": "deepseek-v4-flash-free",`,
		`  "providerID": "opencode",`,
		`  "name": "DeepSeek V4 Flash Free",`,
		`  "status": "active"`,
		"}",
	}, "\n")

	models, err := parseOpenCodeModelsOutput([]byte(out))
	if err != nil {
		t.Fatal(err)
	}
	if len(models) != 2 {
		t.Fatalf("len(models) = %d, want 2 (%+v)", len(models), models)
	}
	if models[0].ID != "opencode/big-pickle" || models[0].Label != "Big Pickle" {
		t.Fatalf("first model = %+v", models[0])
	}
	if models[1].ID != "opencode/deepseek-v4-flash-free" || models[1].Label != "DeepSeek V4 Flash Free" {
		t.Fatalf("second model = %+v", models[1])
	}
}
