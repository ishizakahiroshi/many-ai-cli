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

// secretKeyPattern は `<キー名>: <値>` / `<キー名>=<値>` の値だけを伏せる。
//
// キー名は「末尾の語」で判定する。以前は api_key / auth_token / client_secret /
// secret_key … と完成形を並べていたが、**そこに載っていないキーは例外も警告も無く
// 素通りする**。2026-08-17 の監査 F-61 が挙げた 3 つは、いずれも 1 語ずれていただけで
// 素通りしていた。
//
//	auth_cookie_secret  末尾は secret。client_secret でも secret_key でもない
//	remote_pin_hash     hash が語彙に無い
//	vapid_private_key   末尾は key。api_key でも secret_key でもない
//
// key と hash は単独だと cache_key / commit_hash まで巻き込むので、秘密を示す
// 修飾語が前に付いたときだけ対象にする。それ以外（secret / token / password 等）は
// 修飾語を問わず対象。Redact は外部へ渡る直前の最後の網なので、迷ったら伏せる側に倒す。
//
// なお **本文の第一防御はここではない**。収集側（report.Collect / ExtractAllowedConfig）が
// allowlist で、config.yaml 全文や token に最初から触らない。この正規表現が効くのは
// 添付するセッションログ・hub ログのように、こちらが中身を選べない本文に対してだけ。
const secretKeyPattern = `(?i)(\b[a-z0-9_-]*(?:` +
	// 修飾語を問わず秘密とみなす語
	`secret|token|password|passwd|passphrase|credentials?|` +
	// 秘密を示す修飾語が付いたときだけ対象にする語
	`(?:api|auth|access|client|private|secret|signing|encryption|master|session|refresh|vapid|pin)[_-]?(?:key|hash)` +
	`)\b\s*[:=]\s*)["']?[^\s,"';&?#}]+["']?`

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
	secretKeyRE     = regexp.MustCompile(secretKeyPattern)
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
