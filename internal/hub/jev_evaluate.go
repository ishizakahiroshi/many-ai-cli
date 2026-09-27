package hub

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"io"
	"math"
	"net/http"
	"os"
	"strings"
	"time"
)

// Jev is an explicitly requested, display-only evaluation. It never affects
// provider selection, approvals, or relay state.
const jevEndpoint = "https://api.typesafe.ai/v1/systemone"

var jevTaskTypes = map[string]string{
	"simple_edit":  "A small, precisely scoped edit",
	"bugfix":       "Find and fix a defect when deep investigation is not specified",
	"feature":      "Add a new user-visible behavior",
	"deep_debug":   "Trace a difficult failure across components or platforms",
	"architecture": "Compare or design system-level approaches",
	"research":     "Gather and compare external information",
	"review":       "Assess existing work without implementing it",
}

var jevComplexity = []string{
	"1: Localized and clear, with little uncertainty",
	"2: Bounded change with some unknowns",
	"3: Multiple tradeoffs or substantial context needed",
	"4: Broad investigation or high uncertainty",
}

type jevResult struct {
	TaskType                    string  `json:"task_type"`
	TaskTypeConfidence          float64 `json:"task_type_confidence"`
	Complexity                  float64 `json:"complexity"`
	ComplexityConfidence        float64 `json:"complexity_confidence"`
	NeedsStrongModelProbability float64 `json:"needs_strong_model_probability"`
	InputTokens                 int     `json:"input_tokens"`
	Model                       string  `json:"model"`
}

func (s *Server) handleJevEvaluate(w http.ResponseWriter, r *http.Request) {
	if !s.guard(w, r, http.MethodPost) {
		return
	}
	key := strings.TrimSpace(os.Getenv("TYPESAFE_API_KEY"))
	if key == "" {
		writeJSONError(w, http.StatusServiceUnavailable, "jev_not_configured", "Set TYPESAFE_API_KEY in the Hub process environment")
		return
	}
	var body struct {
		Text string `json:"text"`
	}
	dec := json.NewDecoder(http.MaxBytesReader(w, r.Body, 8192))
	dec.DisallowUnknownFields()
	if err := dec.Decode(&body); err != nil || dec.Decode(&struct{}{}) != io.EOF {
		writeJSONError(w, http.StatusBadRequest, "bad_request", "Expected one JSON object with text")
		return
	}
	text := strings.TrimSpace(body.Text)
	if text == "" || len(text) > 4096 {
		writeJSONError(w, http.StatusBadRequest, "bad_request", "Text must be 1 to 4096 bytes")
		return
	}
	client := newExternalHTTPClient(10 * time.Second)
	client.CheckRedirect = func(*http.Request, []*http.Request) error { return http.ErrUseLastResponse }
	result, err := evaluateJev(r.Context(), client, jevEndpoint, key, text)
	if err != nil {
		// Upstream bodies and request text may contain sensitive data. Never echo them.
		writeJSONError(w, http.StatusBadGateway, "jev_failed", "Jev evaluation failed or returned an unexpected response")
		return
	}
	writeJSON(w, result)
}

func evaluateJev(ctx context.Context, client *http.Client, endpoint, key, text string) (jevResult, error) {
	var zero jevResult
	questions := map[string]any{
		"task_type":          map[string]any{"type": "choice", "instructions": "Choose the main task actually requested. Favor the described work over labels such as 'easy' or 'architecture'.", "criteria": jevTaskTypes},
		"complexity":         map[string]any{"type": "score", "instructions": "Rate the likely scope and uncertainty from the request text alone. Do not assume unseen code is simple.", "criteria": jevComplexity},
		"needs_strong_model": map[string]any{"type": "noul", "instructions": "Would a stronger model likely reduce the risk of task failure, based only on this request?", "criteria": map[string]string{"true": "A stronger model likely reduces failure risk", "false": "There is no clear reason it would reduce failure risk"}},
	}
	payload, err := json.Marshal(map[string]any{"state": text, "model": "jev-latest", "questions": questions})
	if err != nil {
		return zero, err
	}
	req, err := http.NewRequestWithContext(ctx, http.MethodPost, endpoint, bytes.NewReader(payload))
	if err != nil {
		return zero, err
	}
	req.Header.Set("Authorization", "Bearer "+key)
	req.Header.Set("Content-Type", "application/json")
	resp, err := client.Do(req)
	if err != nil {
		return zero, err
	}
	defer resp.Body.Close()
	if resp.StatusCode != http.StatusOK {
		return zero, errors.New("jev upstream status")
	}
	var upstream struct {
		Answers struct {
			TaskType struct {
				Type       string  `json:"type"`
				Choice     string  `json:"choice"`
				Confidence float64 `json:"confidence"`
			} `json:"task_type"`
			Complexity struct {
				Type       string  `json:"type"`
				Score      float64 `json:"score"`
				Confidence float64 `json:"confidence"`
			} `json:"complexity"`
			NeedsStrongModel struct {
				Type string  `json:"type"`
				Noul float64 `json:"noul"`
			} `json:"needs_strong_model"`
		} `json:"answers"`
		Usage struct {
			InputTokens int `json:"input_tokens"`
		} `json:"usage"`
		Model string `json:"model"`
	}
	if err := json.NewDecoder(io.LimitReader(resp.Body, 65536)).Decode(&upstream); err != nil {
		return zero, err
	}
	a := upstream.Answers
	if a.TaskType.Type != "choice" || a.Complexity.Type != "score" || a.NeedsStrongModel.Type != "noul" {
		return zero, errors.New("jev answer types")
	}
	if _, ok := jevTaskTypes[a.TaskType.Choice]; !ok {
		return zero, errors.New("jev task type")
	}
	if !jevProbability(a.TaskType.Confidence) || !jevProbability(a.Complexity.Confidence) || !jevProbability(a.NeedsStrongModel.Noul) || math.IsNaN(a.Complexity.Score) || math.IsInf(a.Complexity.Score, 0) || a.Complexity.Score < 0 || a.Complexity.Score > 3 || upstream.Usage.InputTokens < 0 {
		return zero, errors.New("jev answer range")
	}
	return jevResult{TaskType: a.TaskType.Choice, TaskTypeConfidence: a.TaskType.Confidence, Complexity: a.Complexity.Score + 1, ComplexityConfidence: a.Complexity.Confidence, NeedsStrongModelProbability: a.NeedsStrongModel.Noul, InputTokens: upstream.Usage.InputTokens, Model: upstream.Model}, nil
}

func jevProbability(n float64) bool { return !math.IsNaN(n) && !math.IsInf(n, 0) && n >= 0 && n <= 1 }
