//go:build ignore

// Offline exact Go nextRoutineTime helper from fixed oracle, synthetic dates only.
package main

import (
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"time"
	_ "time/tzdata"
)

type routineSchedule struct {
	Kind     string `json:"kind"`
	Time     string `json:"time,omitempty"`
	Timezone string `json:"timezone,omitempty"`
}

func nextRoutineTime(schedule routineSchedule, after time.Time) (time.Time, error) {
	if schedule.Kind == "manual" {
		return time.Time{}, nil
	}
	if schedule.Kind != "daily" && schedule.Kind != "weekdays" {
		return time.Time{}, errors.New("schedule kind must be manual, daily, or weekdays")
	}
	clock, err := time.Parse("15:04", schedule.Time)
	if err != nil {
		return time.Time{}, errors.New("schedule time must be HH:MM")
	}
	if schedule.Timezone == "" || schedule.Timezone == "Local" {
		return time.Time{}, errors.New("schedule timezone is required")
	}
	loc, err := time.LoadLocation(schedule.Timezone)
	if err != nil {
		return time.Time{}, errors.New("unknown schedule timezone")
	}
	local := after.In(loc)
	for offset := 0; offset < 9; offset++ {
		date := time.Date(local.Year(), local.Month(), local.Day()+offset, 12, 0, 0, 0, loc)
		if schedule.Kind == "weekdays" && (date.Weekday() == time.Saturday || date.Weekday() == time.Sunday) {
			continue
		}
		next := time.Date(date.Year(), date.Month(), date.Day(), clock.Hour(), clock.Minute(), 0, 0, loc)
		if next.Hour() != clock.Hour() || next.Minute() != clock.Minute() || next.Day() != date.Day() {
			continue
		}
		if next.After(after) {
			return next, nil
		}
	}
	return time.Time{}, fmt.Errorf("no next schedule time")
}

type result struct {
	Schedule routineSchedule `json:"schedule"`
	After    string          `json:"after"`
	Next     string          `json:"next"`
	Error    string          `json:"error,omitempty"`
}

func main() {
	out := []result{}
	for _, zone := range []string{"UTC", "Asia/Tokyo", "America/New_York", "Europe/Berlin", "Australia/Lord_Howe", "Pacific/Apia"} {
		for _, after := range []string{"2026-03-08T00:00:00Z", "2026-03-08T08:00:00Z", "2026-10-25T00:00:00Z", "2026-11-01T00:00:00Z", "2026-11-01T06:00:00Z", "2026-10-02T23:00:00Z"} {
			for _, clock := range []string{"01:30", "02:30", "09:00"} {
				for _, kind := range []string{"daily", "weekdays"} {
					c := result{Schedule: routineSchedule{Kind: kind, Time: clock, Timezone: zone}, After: after}
					at, _ := time.Parse(time.RFC3339, after)
					next, err := nextRoutineTime(c.Schedule, at)
					if err != nil {
						c.Error = err.Error()
					} else {
						c.Next = next.UTC().Format(time.RFC3339Nano)
					}
					out = append(out, c)
				}
			}
		}
	}
	for _, sch := range []routineSchedule{{Kind: "manual"}, {Kind: "weekly"}, {Kind: "daily", Time: "1:02", Timezone: "UTC"}, {Kind: "daily", Time: "9:0", Timezone: "UTC"}, {Kind: "daily", Time: "09:00", Timezone: "Local"}, {Kind: "daily", Time: "25:00", Timezone: "UTC"}, {Kind: "daily", Time: "09:00", Timezone: "not/a-zone"}} {
		c := result{Schedule: sch, After: "2026-10-03T00:00:00Z"}
		at, _ := time.Parse(time.RFC3339, c.After)
		next, err := nextRoutineTime(sch, at)
		if err != nil {
			c.Error = err.Error()
		} else {
			c.Next = next.UTC().Format(time.RFC3339Nano)
		}
		out = append(out, c)
	}
	e := json.NewEncoder(os.Stdout)
	e.SetIndent("", "  ")
	if err := e.Encode(out); err != nil {
		panic(err)
	}
}
