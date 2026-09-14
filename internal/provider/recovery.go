package provider

import (
	"crypto/sha256"
	"encoding/hex"
	"fmt"
	"io"
	"os"
	"path/filepath"
	"strings"
	"time"
)

type QuarantineRecord struct {
	OriginalName string `json:"original_name"`
	Quarantined  string `json:"quarantined"`
	DetectedAt   string `json:"detected_at"`
	Reason       string `json:"reason"`
	Digest       string `json:"digest"`
}

func QuarantineFile(path, quarantineRoot, reason string) (QuarantineRecord, error) {
	if strings.TrimSpace(path) == "" || strings.TrimSpace(quarantineRoot) == "" {
		return QuarantineRecord{}, fmt.Errorf("quarantine path is required")
	}
	input, err := os.Open(path)
	if err != nil {
		return QuarantineRecord{}, err
	}
	defer input.Close()
	if err := os.MkdirAll(quarantineRoot, 0o700); err != nil {
		return QuarantineRecord{}, err
	}
	stamp := time.Now().UTC().Format("20060102T150405.000000000Z")
	name := stamp + "-" + filepath.Base(path)
	target := filepath.Join(quarantineRoot, name)
	output, err := os.OpenFile(target, os.O_CREATE|os.O_EXCL|os.O_WRONLY, 0o600)
	if err != nil {
		return QuarantineRecord{}, err
	}
	hash := sha256.New()
	if _, err := io.Copy(io.MultiWriter(output, hash), input); err != nil {
		_ = output.Close()
		return QuarantineRecord{}, err
	}
	if err := output.Close(); err != nil {
		return QuarantineRecord{}, err
	}
	return QuarantineRecord{
		OriginalName: filepath.Base(path),
		Quarantined:  target,
		DetectedAt:   time.Now().UTC().Format(time.RFC3339Nano),
		Reason:       reason,
		Digest:       hex.EncodeToString(hash.Sum(nil)),
	}, nil
}
