//go:build ignore

// Fixed Go oracle 21d0bc7935a2c4696fb89ccff2e324157a528c2d.
// Record declarations copied from internal/hub/routine_store.go without edits.
// Run by exact filename; all inputs below are synthetic and memory-only.
package main

import (
	"encoding/json"
	"os"
)

type routineSchedule struct {
	Kind     string `json:"kind"`
	Time     string `json:"time,omitempty"`
	Timezone string `json:"timezone,omitempty"`
}

type routine struct {
	ID             string          `json:"id"`
	Name           string          `json:"name"`
	CWD            string          `json:"cwd"`
	Provider       string          `json:"provider"`
	Model          string          `json:"model,omitempty"`
	Prompt         string          `json:"prompt"`
	Enabled        bool            `json:"enabled"`
	Schedule       routineSchedule `json:"schedule"`
	NextRunAt      string          `json:"next_run_at,omitempty"`
	CreatedAt      string          `json:"created_at"`
	UpdatedAt      string          `json:"updated_at"`
	CompletionMode string          `json:"completion_mode"`
}

type routineRun struct {
	ID              string `json:"id"`
	RoutineID       string `json:"routine_id"`
	RoutineName     string `json:"routine_name"`
	CWD             string `json:"cwd"`
	Provider        string `json:"provider"`
	Model           string `json:"model,omitempty"`
	Prompt          string `json:"prompt"`
	Trigger         string `json:"trigger"`
	Status          string `json:"status"`
	SessionID       int    `json:"session_id,omitempty"`
	SessionDBID     int64  `json:"session_db_id,omitempty"`
	HubInstanceID   string `json:"hub_instance_id,omitempty"`
	StartedAt       string `json:"started_at"`
	UpdatedAt       string `json:"updated_at"`
	FinishedAt      string `json:"finished_at,omitempty"`
	Summary         string `json:"summary,omitempty"`
	Result          string `json:"result,omitempty"`
	ResultTruncated bool   `json:"result_truncated,omitempty"`
	ResultAvailable bool   `json:"result_available"`
	Error           string `json:"error,omitempty"`
	RequestID       string `json:"request_id,omitempty"`
	SessionLabel    string `json:"session_label"`
}

type routineFile struct {
	Version  int               `json:"version"`
	Routines []routine         `json:"routines"`
	Runs     []routineRun      `json:"runs"`
	Requests map[string]string `json:"requests,omitempty"`
}

func main() {
	inputs := []string{
		`null`, `{}`, `{"version":null,"routines":null,"runs":null,"requests":null}`,
		`{"Version":1,"Routines":[{"ID":"historical","Provider":"retired","Schedule":{"Kind":"old"}}]}`,
		`{"version":1,"routines":[{"name":"old","name":null,"enabled":true,"enabled":null,"schedule":{"kind":"daily"},"schedule":{"timezone":"UTC"}}]}`,
		`{"version":1,"runs":[{"id":"run","session_id":42,"session_db_id":700,"result_available":false,"result_truncated":false}]}`,
		`{"version":1,"requests":{"a":"first","a":"last","empty":null}}`,
		`{"version":1,"unknown":1e999}`, `{"version":1,"runs":false}`,
		`{"version":1,"routines":[{"name":7,"name":"later"}]}`,
		`{"version":1} {"version":2}`, `{"version":2}`,
	}
	type Case struct {
		Input  string          `json:"input"`
		OK     bool            `json:"ok"`
		Output json.RawMessage `json:"output,omitempty"`
	}
	out := []Case{}
	for _, input := range inputs {
		data := routineFile{Version: 1, Routines: []routine{}, Runs: []routineRun{}}
		err := json.Unmarshal([]byte(input), &data)
		c := Case{Input: input, OK: err == nil}
		if err == nil {
			c.Output, _ = json.Marshal(data)
		}
		out = append(out, c)
	}
	json.NewEncoder(os.Stdout).Encode(out)
}
