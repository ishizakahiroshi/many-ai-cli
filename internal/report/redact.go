package report

import (
	"net"
	"regexp"
	"sort"
	"strconv"
	"strings"

	"many-ai-cli/internal/config"
)

const (
	redactedSecret      = "<REDACTED_SECRET>"
	redactedPrivatePath = "<REDACTED_PRIVATE_PATH>"
	redactedIP          = "<REDACTED_IP>"
	redactedEmail       = "<REDACTED_EMAIL>"
	redactedHost        = "<REDACTED_HOST>"
)

var (
	// ドライブレターは固定しない。開発ルートが C: 以外（実例: D:\dev への移設）へ
	// 移った瞬間に伏せ字が外れ、kb / .ssh / github\private の実パスがバグレポートへ
	// そのまま載る。伏せ字はどのドライブに置かれていても効かなければならない。
	privateWindowsPathRE = regexp.MustCompile(`(?i)[a-z]:[\\/]+dev[\\/]+(?:kb|\.ssh|github[\\/]+private)(?:[\\/]+[^\s"'<>|]*)?`)
	privateUnixPathRE    = regexp.MustCompile(`(?i)/(?:srv/)?dev/(?:kb|\.ssh|github/private)(?:/[^\s"'<>|]*)?`)
	windowsHomeRE        = regexp.MustCompile(`(?i)[a-z]:[\\/]+users[\\/]+[^\\/\s]+[\\/]+`)
	unixHomeRE           = regexp.MustCompile(`/(?:home|Users)/[^/\s]+/`)

	queryTokenRE    = regexp.MustCompile(`(?i)([?&](?:access_)?token=)[^&#\s"']+`)
	bearerRE        = regexp.MustCompile(`(?i)((?:authorization\s*:\s*)?bearer\s+)[^\s,"']+`)
	secretKeyRE     = regexp.MustCompile(`(?i)(\b(?:[a-z0-9]+_)*(?:api_key|auth_token|api_token|access_token|client_secret|password|passwd|secret_key|token)\b\s*[:=]\s*)["']?[^\s,"';&?#}]+["']?`)
	jwtRE           = regexp.MustCompile(`\beyJ[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+\b`)
	credentialURLRE = regexp.MustCompile(`([a-zA-Z][a-zA-Z0-9+.\-]*://[^\s:/@]+:)([^\s/@]+)(@)`)
	privateKeyRE    = regexp.MustCompile(`(?s)-----BEGIN [A-Z0-9 ]*PRIVATE KEY-----.*?-----END [A-Z0-9 ]*PRIVATE KEY-----`)

	emailRE       = regexp.MustCompile(`[A-Za-z0-9.!#$%&'*+/=?^_` + "`" + `{|}~-]+@[A-Za-z0-9](?:[A-Za-z0-9-]{0,61}[A-Za-z0-9])?(?:\.[A-Za-z0-9](?:[A-Za-z0-9-]{0,61}[A-Za-z0-9])?)+`)
	publicEmailRE = regexp.MustCompile(`(?i)ishizakahiroshi\.dev@gmail\.com`)
	ipv4RE        = regexp.MustCompile(`\b(?:[0-9]{1,3}\.){3}[0-9]{1,3}\b`)
	ipv6RE        = regexp.MustCompile(`(?i)[0-9a-f]*:[0-9a-f:]+`)
	familyHostRE  = regexp.MustCompile(`(?i)\bishiz\.[a-z0-9.-]+\b`)
)

var knownSecretPatterns = []*regexp.Regexp{
	regexp.MustCompile(`\bsk-(?:ant-api[0-9]+-)?[A-Za-z0-9_-]{10,}\b`),
	regexp.MustCompile(`\b(?:ghp_|gho_|ghu_|ghs_|ghr_|github_pat_)[A-Za-z0-9_]{10,}\b`),
	regexp.MustCompile(`\bglpat-[A-Za-z0-9_-]{10,}\b`),
	regexp.MustCompile(`\bxox[abprs]-[A-Za-z0-9-]{10,}\b`),
	regexp.MustCompile(`\bAIza[A-Za-z0-9_-]{10,}\b`),
	regexp.MustCompile(`\bhf_[A-Za-z0-9]{10,}\b`),
	regexp.MustCompile(`\bnpm_[A-Za-z0-9]{10,}\b`),
	regexp.MustCompile(`\bpypi-[A-Za-z0-9_-]{10,}\b`),
	regexp.MustCompile(`\bxai-[A-Za-z0-9_-]{10,}\b`),
	regexp.MustCompile(`\bgsk_[A-Za-z0-9]{10,}\b`),
	regexp.MustCompile(`\b(?:AKIA|ASIA|AROA)[A-Z0-9]{16}\b`),
}

var allowedProviders = []string{
	"claude",
	"codex",
	"copilot",
	"cursor-agent",
	"opencode",
	"grok",
}

// Redact removes secrets and private machine details before report content can
// leave the local Hub. It is intentionally conservative: false positives are
// preferable to leaking credentials or personal infrastructure details.
func Redact(s string) string {
	s = privateWindowsPathRE.ReplaceAllString(s, redactedPrivatePath)
	s = privateUnixPathRE.ReplaceAllString(s, redactedPrivatePath)
	s = normalizeHomeDir(s)

	s = queryTokenRE.ReplaceAllString(s, "${1}"+redactedSecret)
	s = bearerRE.ReplaceAllString(s, "${1}"+redactedSecret)
	s = secretKeyRE.ReplaceAllString(s, "${1}"+redactedSecret)
	s = credentialURLRE.ReplaceAllString(s, "${1}"+redactedSecret+"${3}")
	s = jwtRE.ReplaceAllString(s, redactedSecret)
	for _, re := range knownSecretPatterns {
		s = re.ReplaceAllString(s, redactedSecret)
	}
	s = privateKeyRE.ReplaceAllString(s, redactedSecret)

	s = redactIPs(s)
	const publicEmailPlaceholder = "MANYAICLI_PUBLIC_EMAIL_PLACEHOLDER"
	s = publicEmailRE.ReplaceAllString(s, publicEmailPlaceholder)
	s = emailRE.ReplaceAllString(s, redactedEmail)
	s = strings.ReplaceAll(s, publicEmailPlaceholder, "ishizakahiroshi.dev@gmail.com")
	return familyHostRE.ReplaceAllString(s, redactedHost)
}

// normalizeHomeDir removes the local account name without consulting the
// current process home. Reports may contain paths produced on another OS.
func normalizeHomeDir(s string) string {
	s = windowsHomeRE.ReplaceAllString(s, "~/")
	return unixHomeRE.ReplaceAllString(s, "~/")
}

func redactIPs(s string) string {
	s = ipv4RE.ReplaceAllStringFunc(s, redactIP)
	return ipv6RE.ReplaceAllStringFunc(s, redactIP)
}

func redactIP(candidate string) string {
	ip := net.ParseIP(candidate)
	if ip == nil || ip.IsLoopback() {
		return candidate
	}
	return redactedIP
}

// ExtractAllowedConfig returns only fields explicitly approved for bug report
// environment metadata. It must not be replaced with reflection or config
// serialization: Config also contains authentication and private endpoint data.
func ExtractAllowedConfig(cfg *config.Config) map[string]string {
	allowed := make(map[string]string)
	if cfg == nil {
		return allowed
	}

	allowed["hub_port"] = strconv.Itoa(cfg.Hub.Port)
	providers := make([]string, 0, len(allowedProviders))
	for _, provider := range allowedProviders {
		model := strings.TrimSpace(cfg.UserPrefs.Spawn.LastModel[provider])
		if model == "" {
			continue
		}
		providers = append(providers, provider)
		allowed["model."+provider] = Redact(model)
	}
	sort.Strings(providers)
	if len(providers) > 0 {
		allowed["providers"] = strings.Join(providers, ",")
	}
	return allowed
}
