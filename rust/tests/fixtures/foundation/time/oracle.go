package main

import (
	"encoding/json"
	"os"
	"time"
)

func main() {
	formats := []map[string]any{}
	for _, s := range []int64{-62135596800, -1, 0, 1700000000, 4102444800} {
		for _, ns := range []int64{0, 1, 120000000, 999999999} {
			for _, off := range []int{0, 32400, -19800, 1234, -59} {
				t := time.Unix(s, ns).In(time.FixedZone("synthetic", off))
				formats = append(formats, map[string]any{"seconds": s, "nanos": ns, "offset": off, "seconds_text": t.Format(time.RFC3339), "nano_text": t.Format(time.RFC3339Nano)})
			}
		}
	}
	parses := []map[string]any{}
	for _, s := range []string{"0001-01-01T00:00:00Z", "0000-01-01T00:00:00Z", "2026-10-03T05:00:00Z", "2026-10-03T05:00:00.123456789123Z", "2026-10-03T05:00:00,12+09:00", "2026-10-03T5:00:00Z", "2026-10-03T05:00:00+24:00", "2026-10-03T05:00:00+24:60", "2026-10-03T05:00:00+25:00", "2026-10-03T05:00:00-00:60", "2026-02-29T05:00:00Z", "2024-02-29T05:00:00Z", "2026-10-03T05:00:60Z", "2026-10-03T05:00:00z", "2026-10-03 05:00:00Z", "2026-10-03T05:00:00.Z", "2026-10-03T05:00:00Z trailing", "2026-10-03T24:00:00Z"} {
		t, e := time.Parse(time.RFC3339, s)
		r := map[string]any{"input": s, "error": e != nil}
		if e == nil {
			r["seconds"] = t.Unix()
			r["nanos"] = t.Nanosecond()
		}
		parses = append(parses, r)
	}
	e := json.NewEncoder(os.Stdout)
	e.SetIndent("", "  ")
	if e.Encode(map[string]any{"formats": formats, "parses": parses}) != nil {
		os.Exit(1)
	}
}
