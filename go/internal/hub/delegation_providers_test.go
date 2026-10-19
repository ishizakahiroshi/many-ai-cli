package hub

import (
	"strings"
	"testing"

	"many-ai-cli/internal/wrapper"
)

// 委譲案内は「子に使える provider はこれだけ、ほかは受け付けない」と AI に教える。
// 一覧は orchestrationProviders と案内の本文の 2 か所にあるので、Hub が受け付ける
// provider がすべて案内に載っていることをここで確かめる（command-code を足したとき、
// 案内だけ 6 つのまま取り残された）。
func TestDelegationPromptListsEveryOrchestrationProvider(t *testing.T) {
	text := wrapper.DelegationPromptText()
	for _, provider := range orchestrationProviders {
		if !strings.Contains(text, provider) {
			t.Errorf("delegation prompt does not list provider %q accepted by orchestrate spawn", provider)
		}
	}
}
