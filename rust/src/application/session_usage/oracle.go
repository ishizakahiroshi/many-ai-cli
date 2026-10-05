package main
import("encoding/json";"os")
type modelPricing struct {
	InputPerMTok      float64 // USD per 1M input tokens
	OutputPerMTok     float64 // USD per 1M output tokens
	CacheReadPerMTok  float64 // USD per 1M cache-read tokens (0 = cache なし or 同 input)
	CacheWritePerMTok float64 // USD per 1M cache-write tokens (0 = 計上しない)
}

// modelPriceTable: モデル ID（完全一致または前方一致で検索）→ 単価。
// Codex 用: トークン → コスト算出に使う。
// Claude 用: relay が cost をそのまま送るため原則使わないが、
//
//	将来的にトークン内訳を表示する場合に備えて収録。
var modelPriceTable = map[string]modelPricing{
	// --- OpenAI / Codex ---
	// gpt-4.1 系 (2025-04 発表)
	"gpt-4.1":      {InputPerMTok: 2.00, OutputPerMTok: 8.00, CacheReadPerMTok: 0.50},
	"gpt-4.1-mini": {InputPerMTok: 0.40, OutputPerMTok: 1.60, CacheReadPerMTok: 0.10},
	"gpt-4.1-nano": {InputPerMTok: 0.10, OutputPerMTok: 0.40, CacheReadPerMTok: 0.025},
	"gpt-4o":       {InputPerMTok: 2.50, OutputPerMTok: 10.00, CacheReadPerMTok: 1.25},
	"gpt-4o-mini":  {InputPerMTok: 0.15, OutputPerMTok: 0.60, CacheReadPerMTok: 0.075},
	"gpt-5":        {InputPerMTok: 10.00, OutputPerMTok: 40.00, CacheReadPerMTok: 2.50},
	"gpt-5.5":      {InputPerMTok: 10.00, OutputPerMTok: 40.00, CacheReadPerMTok: 2.50},
	"o3":           {InputPerMTok: 10.00, OutputPerMTok: 40.00, CacheReadPerMTok: 2.50},
	"o4-mini":      {InputPerMTok: 1.10, OutputPerMTok: 4.40, CacheReadPerMTok: 0.275},
	// --- Anthropic / Claude ---
	// Claude 4 系 (2026 Q1)
	"claude-opus-4":   {InputPerMTok: 15.00, OutputPerMTok: 75.00, CacheReadPerMTok: 1.50, CacheWritePerMTok: 18.75},
	"claude-sonnet-4": {InputPerMTok: 3.00, OutputPerMTok: 15.00, CacheReadPerMTok: 0.30, CacheWritePerMTok: 3.75},
	"claude-haiku-4":  {InputPerMTok: 0.80, OutputPerMTok: 4.00, CacheReadPerMTok: 0.08, CacheWritePerMTok: 1.00},
	// 現行モデル（2026-06 公式単価。CacheRead=入力×0.1 / CacheWrite=入力×1.25 の慣習で算出）
	// audit #31: 現行モデル ID を正規単価で表に収録（claude-api スキル公式テーブル 2026-06-04 確認）。
	"claude-fable-5":    {InputPerMTok: 10.00, OutputPerMTok: 50.00, CacheReadPerMTok: 1.00, CacheWritePerMTok: 12.50},
	"claude-opus-4-8":   {InputPerMTok: 5.00, OutputPerMTok: 25.00, CacheReadPerMTok: 0.50, CacheWritePerMTok: 6.25},
	"claude-opus-4-7":   {InputPerMTok: 5.00, OutputPerMTok: 25.00, CacheReadPerMTok: 0.50, CacheWritePerMTok: 6.25},
	"claude-opus-4-6":   {InputPerMTok: 5.00, OutputPerMTok: 25.00, CacheReadPerMTok: 0.50, CacheWritePerMTok: 6.25},
	"claude-sonnet-4-6": {InputPerMTok: 3.00, OutputPerMTok: 15.00, CacheReadPerMTok: 0.30, CacheWritePerMTok: 3.75},
	// claude-opus-4-5 / claude-opus-4（4.0）: 公式 pricing 表に現行単価の記載なし（legacy/deprecated）。
	// 確証なしのため $15/$75 据え置き（C6 判断ログ参照）。
	"claude-opus-4-5":   {InputPerMTok: 15.00, OutputPerMTok: 75.00, CacheReadPerMTok: 1.50, CacheWritePerMTok: 18.75},
	"claude-sonnet-4-5": {InputPerMTok: 3.00, OutputPerMTok: 15.00, CacheReadPerMTok: 0.30, CacheWritePerMTok: 3.75},
	"claude-haiku-4-5":  {InputPerMTok: 1.00, OutputPerMTok: 5.00, CacheReadPerMTok: 0.10, CacheWritePerMTok: 1.25},
	"claude-3-5-sonnet": {InputPerMTok: 3.00, OutputPerMTok: 15.00, CacheReadPerMTok: 0.30, CacheWritePerMTok: 3.75},
	"claude-3-5-haiku":  {InputPerMTok: 0.80, OutputPerMTok: 4.00, CacheReadPerMTok: 0.08, CacheWritePerMTok: 1.00},
	"claude-3-opus":     {InputPerMTok: 15.00, OutputPerMTok: 75.00, CacheReadPerMTok: 1.50, CacheWritePerMTok: 18.75},
	"claude-3-sonnet":   {InputPerMTok: 3.00, OutputPerMTok: 15.00, CacheReadPerMTok: 0.30, CacheWritePerMTok: 3.75},
	"claude-3-haiku":    {InputPerMTok: 0.25, OutputPerMTok: 1.25, CacheReadPerMTok: 0.03, CacheWritePerMTok: 0.30},
}

// lookupModelPricing はモデル ID（完全一致 → 前方一致の順）で価格表を引く。
// ヒットしない場合は (modelPricing{}, false) を返す。
// Codex は "gpt-4.1 medium" のように effort サフィックスが付く場合があるため
// スペース以前のプレフィックスでも検索する。
func lookupModelPricing(modelID string) (modelPricing, bool) {
	if p, ok := modelPriceTable[modelID]; ok {
		return p, true
	}
	// スペースで区切った最初のトークンで再試行（effort サフィックス除去）
	for i, c := range modelID {
		if c == ' ' {
			if p, ok := modelPriceTable[modelID[:i]]; ok {
				return p, true
			}
			break
		}
	}
	return modelPricing{}, false
}

// calcCostUSD はトークン数と価格表からコスト（USD）を算出する。
// 価格表にモデルが無い場合は (0, false) を返す。
func calcCostUSD(modelID string, tokIn, tokOut, tokCacheRead int) (cost float64, known bool) {
	p, ok := lookupModelPricing(modelID)
	if !ok {
		return 0, false
	}
	// Codex reports cached input as a subset of input_tokens. Charge the
	// uncached remainder at the normal input rate and the cached subset at the
	// cache-read rate. Clamp malformed counts so cache > input cannot produce a
	// negative billable input amount.
	if tokIn < 0 {
		tokIn = 0
	}
	if tokOut < 0 {
		tokOut = 0
	}
	if tokCacheRead < 0 {
		tokCacheRead = 0
	}
	if tokCacheRead > tokIn {
		tokCacheRead = tokIn
	}
	uncachedInput := tokIn - tokCacheRead
	const mTok = 1_000_000.0
	cost = float64(uncachedInput)*p.InputPerMTok/mTok +
		float64(tokOut)*p.OutputPerMTok/mTok +
		float64(tokCacheRead)*p.CacheReadPerMTok/mTok
	return cost, true
}

type Input struct{Model string `json:"model"`;In int `json:"input"`;Out int `json:"output"`;Cache int `json:"cache"`}
type Output struct{Cost float64 `json:"cost"`;Known bool `json:"known"`}
func main(){var in []Input;if err:=json.NewDecoder(os.Stdin).Decode(&in);err!=nil{panic(err)};out:=make([]Output,0,len(in));for _,v:=range in{c,k:=calcCostUSD(v.Model,v.In,v.Out,v.Cache);out=append(out,Output{c,k})};if err:=json.NewEncoder(os.Stdout).Encode(out);err!=nil{panic(err)}}