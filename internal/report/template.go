package report

import (
	"fmt"
	"sort"
	"strings"
)

// TemplateInput is shared by the Web dashboard and the issue CLI.
type TemplateInput struct {
	Locale              string
	Symptom             string
	Reproduction        string
	EnvironmentMarkdown string
	Environment         Environment
}

// RenderEnvironment renders an editable allowlisted environment section.
func RenderEnvironment(env Environment, locale string) string {
	labels := environmentLabels(locale)
	lines := make([]string, 0, 10)
	appendValue := func(label, value string) {
		if value != "" {
			lines = append(lines, fmt.Sprintf("- %s: `%s`", label, escapeInlineCode(cleanValue(value))))
		}
	}
	appendValue(labels.version, env.Version)
	appendValue(labels.os, env.OS)
	appendValue(labels.architecture, env.Architecture)
	appendValue(labels.goVersion, env.GoVersion)
	appendValue(labels.provider, env.Provider)
	appendValue(labels.model, env.Model)
	appendValue(labels.userAgent, env.UserAgent)

	keys := make([]string, 0, len(env.AllowedConfig))
	for key := range env.AllowedConfig {
		keys = append(keys, key)
	}
	sort.Strings(keys)
	for _, key := range keys {
		appendValue(key, env.AllowedConfig[key])
	}
	return Redact(strings.Join(lines, "\n"))
}

// RenderMarkdown builds the complete text shown to the user before GitHub is
// opened. It does not skip empty sections, so the preview remains explicit.
func RenderMarkdown(input TemplateInput) string {
	ja := normalizeLocale(input.Locale) == "ja"
	symptomHeading, reproductionHeading, environmentHeading := "Symptom", "Steps to reproduce (optional)", "Environment"
	noReproduction := "Not provided"
	if ja {
		symptomHeading, reproductionHeading, environmentHeading = "症状", "再現手順（任意）", "環境情報"
		noReproduction = "未記入"
	}
	symptom := strings.TrimSpace(input.Symptom)
	reproduction := strings.TrimSpace(input.Reproduction)
	if reproduction == "" {
		reproduction = noReproduction
	}
	environment := strings.TrimSpace(input.EnvironmentMarkdown)
	if environment == "" {
		environment = RenderEnvironment(input.Environment, input.Locale)
	}
	body := fmt.Sprintf("## %s\n\n%s\n\n## %s\n\n%s\n\n## %s\n\n%s\n",
		symptomHeading, symptom, reproductionHeading, reproduction, environmentHeading, environment)
	return Redact(body)
}

// DefaultTitle derives a short title without adding report content that was
// not visible in the symptom field.
func DefaultTitle(symptom string) string {
	first := strings.TrimSpace(strings.Split(strings.ReplaceAll(symptom, "\r\n", "\n"), "\n")[0])
	first = strings.Join(strings.Fields(first), " ")
	if len([]rune(first)) > 80 {
		first = string([]rune(first)[:80]) + "…"
	}
	if first == "" {
		return "Bug report"
	}
	return "Bug: " + Redact(first)
}

type envLabels struct {
	version, os, architecture, goVersion, provider, model, userAgent string
}

func environmentLabels(locale string) envLabels {
	if normalizeLocale(locale) == "ja" {
		return envLabels{"many-ai-cli バージョン", "OS", "アーキテクチャ", "Go バージョン", "Provider", "モデル", "ブラウザ"}
	}
	return envLabels{"many-ai-cli version", "OS", "Architecture", "Go version", "Provider", "Model", "Browser"}
}

func normalizeLocale(locale string) string {
	if strings.HasPrefix(strings.ToLower(strings.TrimSpace(locale)), "ja") {
		return "ja"
	}
	return "en"
}

func escapeInlineCode(value string) string {
	return strings.ReplaceAll(value, "`", "'")
}
