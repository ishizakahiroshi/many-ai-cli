package hub

import (
	"reflect"
	"testing"
)

func TestExtractDoneSummaryTextsReturnsEveryCompleteBlock(t *testing.T) {
	data := []byte(
		"prefix [MANY-AI-CLI-DONE] first summary [/MANY-AI-CLI-DONE]" +
			" middle [MANY-AI-CLI-DONE] second summary [/MANY-AI-CLI-DONE] suffix",
	)
	got := extractDoneSummaryTexts(data)
	want := []string{"first summary", "second summary"}
	if !reflect.DeepEqual(got, want) {
		t.Fatalf("extractDoneSummaryTexts = %#v, want %#v", got, want)
	}
}

func TestExtractDoneSummaryTextsIgnoresEmptyAndIncompleteBlocks(t *testing.T) {
	data := []byte(
		"[MANY-AI-CLI-DONE]   [/MANY-AI-CLI-DONE]" +
			"[MANY-AI-CLI-DONE] complete [/MANY-AI-CLI-DONE]" +
			"[MANY-AI-CLI-DONE] incomplete",
	)
	got := extractDoneSummaryTexts(data)
	want := []string{"complete"}
	if !reflect.DeepEqual(got, want) {
		t.Fatalf("extractDoneSummaryTexts = %#v, want %#v", got, want)
	}
}
