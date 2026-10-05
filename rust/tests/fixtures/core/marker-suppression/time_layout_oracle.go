// Synthetic timestamp layout observations; no Hub, providers, or account data.
package main

import (
	"fmt"
	"time"
)

func main() {
	for _, year := range []int{-1, 0, 1, 9999, 10000} {
		at := time.Date(year, 1, 1, 0, 0, 0, 123400000, time.UTC)
		fmt.Printf("%d %s %s\n", year, at.Format(time.RFC3339), at.Format(time.RFC3339Nano))
	}
	for _, offset := range []int{-86400, -30, 30, 86400} {
		at := time.Unix(0, 1).In(time.FixedZone("synthetic", offset))
		fmt.Printf("offset=%d %s %s\n", offset, at.Format(time.RFC3339), at.Format(time.RFC3339Nano))
	}
}
