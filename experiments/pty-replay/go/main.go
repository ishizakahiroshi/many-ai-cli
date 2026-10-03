// Standalone comparison harness. The product replay implementation is in
// pty_replay.go; this file deliberately keeps its oracle independent of it.
package main

import (
	"bufio"
	"bytes"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"os"
	"path/filepath"
	"runtime"
	"strconv"
	"strings"
	"sync/atomic"
	"time"
)

const (
	ptyReplayBufferLimit = 2 * 1024 * 1024
	maxCount = uint64(1<<63 - 1)
	pacedOperations = 1500
	pacedInterval = 20 * time.Millisecond
	warmupDuration = 3 * time.Second
)

type options struct {
	mode string
	fixtureDir string
	repetitions uint64
}

type fixture struct {
	payload []byte
	chunks [][]byte
}

type doneRecord struct {
	Event string `json:"event"`
	Language string `json:"language"`
	Mode string `json:"mode"`
	WallMS float64 `json:"wall_ms"`
	SnapshotMS float64 `json:"snapshot_ms"`
	ExpectedBytes uint64 `json:"expected_bytes"`
	CompletedBytes uint64 `json:"completed_bytes"`
	ExpectedOps uint64 `json:"expected_ops"`
	CompletedOps uint64 `json:"completed_ops"`
	Total int64 `json:"total"`
	OutputHash string `json:"output_hash,omitempty"`
	OK bool `json:"ok,omitempty"`
	MaxLatenessMS float64 `json:"max_lateness_ms"`
}

func main() {
	if err := run(); err != nil {
		fmt.Fprintln(os.Stderr, "replay-bench-go:", err)
		os.Exit(1)
	}
}

func run() error {
	if strconv.IntSize != 64 {
		return errors.New("a 64-bit executable is required")
	}
	opts, err := parseOptions(os.Args[1:])
	if err != nil {
		return err
	}
	out := bufio.NewWriter(os.Stdout)
	if opts.mode == "verify" {
		return verify(opts.fixtureDir, out)
	}
	return measure(opts, bufio.NewReader(os.Stdin), out)
}

func parseOptions(args []string) (options, error) {
	o := options{repetitions: 1}
	seen := make(map[string]bool)
	for i := 0; i < len(args); i += 2 {
		key := args[i]
		if i+1 == len(args) || seen[key] {
			return o, fmt.Errorf("missing value or duplicate option: %q", key)
		}
		seen[key] = true
		switch key {
		case "--mode":
			o.mode = args[i+1]
		case "--fixture-dir":
			o.fixtureDir = args[i+1]
		case "--repetitions":
			value, err := decimal(args[i+1])
			if err != nil || value == 0 {
				return o, errors.New("repetitions must be a positive decimal int64")
			}
			o.repetitions = value
		default:
			return o, fmt.Errorf("unknown option: %q", key)
		}
	}
	if o.mode != "verify" && o.mode != "paced" && o.mode != "saturated" {
		return o, errors.New("--mode must be verify, paced, or saturated")
	}
	if o.fixtureDir == "" {
		return o, errors.New("--fixture-dir is required")
	}
	return o, nil
}

func decimal(s string) (uint64, error) {
	if s == "" {
		return 0, errors.New("empty decimal")
	}
	for _, c := range []byte(s) {
		if c < '0' || c > '9' {
			return 0, errors.New("nondecimal character")
		}
	}
	return strconv.ParseUint(s, 10, 63)
}

func safeName(s string) bool {
	if s == "" || s == "." || s == ".." || strings.HasSuffix(s, ".") {
		return false
	}
	for _, c := range []byte(s) {
		if !((c >= 'a' && c <= 'z') || (c >= 'A' && c <= 'Z') ||
			(c >= '0' && c <= '9') || c == '_' || c == '-' || c == '.') {
			return false
		}
	}
	return true
}

func readLines(path string) ([]string, error) {
	data, err := os.ReadFile(path)
	if err != nil {
		return nil, err
	}
	// Both LF and CRLF are accepted; blank records and a UTF-8 BOM are not.
	text := strings.ReplaceAll(string(data), "\r\n", "\n")
	text = strings.TrimSuffix(text, "\n")
	if text == "" {
		return nil, fmt.Errorf("empty records file: %s", filepath.Base(path))
	}
	lines := strings.Split(text, "\n")
	for _, line := range lines {
		if line == "" || strings.ContainsRune(line, '\r') {
			return nil, fmt.Errorf("blank or malformed record: %s", filepath.Base(path))
		}
	}
	return lines, nil
}

func loadFixture(dir, payloadName, chunksName string, allowZero bool) (fixture, error) {
	var f fixture
	if !safeName(payloadName) || !safeName(chunksName) {
		return f, errors.New("fixture paths must be simple ASCII filenames")
	}
	var err error
	f.payload, err = os.ReadFile(filepath.Join(dir, payloadName))
	if err != nil {
		return f, err
	}
	lines, err := readLines(filepath.Join(dir, chunksName))
	if err != nil {
		return f, err
	}
	position := 0
	for _, line := range lines {
		n, err := decimal(line)
		if err != nil || (!allowZero && n == 0) || n > uint64(len(f.payload)-position) {
			return f, fmt.Errorf("invalid boundary in %s", chunksName)
		}
		end := position + int(n)
		f.chunks = append(f.chunks, f.payload[position:end])
		position = end
	}
	if position != len(f.payload) {
		return f, fmt.Errorf("boundary sum differs from payload length: %s", payloadName)
	}
	return f, nil
}

// Checkpoints compare every byte and total, then corrupt the returned copy and
// compare again. The oracle is solely the concatenated input prefix's suffix.
func checkSnapshot(replay *ptyReplayBuffer, prefix []byte) error {
	want := prefix
	if len(want) > ptyReplayBufferLimit {
		want = want[len(want)-ptyReplayBufferLimit:]
	}
	got, total := replay.snapshot()
	if total != int64(len(prefix)) || !bytes.Equal(got, want) {
		return errors.New("snapshot bytes or total differ from concat-and-tail oracle")
	}
	for i := range got {
		got[i] ^= 0xff
	}
	got, total = replay.snapshot()
	if total != int64(len(prefix)) || !bytes.Equal(got, want) {
		return errors.New("snapshot aliases retained bytes")
	}
	return nil
}

func verify(dir string, out *bufio.Writer) error {
	lines, err := readLines(filepath.Join(dir, "cases.tsv"))
	if err != nil {
		return err
	}
	names := make(map[string]bool)
	checkpoints := 0
	for _, line := range lines {
		fields := strings.Split(line, "\t")
		if len(fields) != 3 || !safeName(fields[0]) || names[fields[0]] {
			return errors.New("invalid or duplicate cases.tsv record")
		}
		names[fields[0]] = true
		f, err := loadFixture(dir, fields[1], fields[2], true)
		if err != nil {
			return fmt.Errorf("case %s: %w", fields[0], err)
		}
		var replay ptyReplayBuffer
		if err := checkSnapshot(&replay, nil); err != nil {
			return fmt.Errorf("case %s initial: %w", fields[0], err)
		}
		checkpoints++
		position := 0
		for i, chunk := range f.chunks {
			input := bytes.Clone(chunk)
			replay.append(input)
			for j := range input {
				input[j] ^= 0xff
			}
			position += len(chunk)
			if i < 16 || (i+1)%64 == 0 || i+1 == len(f.chunks) {
				if err := checkSnapshot(&replay, f.payload[:position]); err != nil {
					return fmt.Errorf("case %s chunk %d: %w", fields[0], i+1, err)
				}
				checkpoints++
			}
		}
	}
	if err := verifyConcurrent(); err != nil {
		return fmt.Errorf("concurrency: %w", err)
	}
	return emit(out, struct {
		Event string `json:"event"`
		Language string `json:"language"`
		OK bool `json:"ok"`
		Cases int `json:"cases"`
		Checkpoints int `json:"checkpoints"`
		ConcurrencyOK bool `json:"concurrency_ok"`
	}{"VERIFY", "go", true, len(lines), checkpoints, true})
}

func verifyConcurrent() error {
	const chunkSize = 4096
	const operations = 4096
	const finalTotal = int64(chunkSize * operations)
	var replay ptyReplayBuffer
	var finished atomic.Bool
	firstAppended := make(chan struct{})
	firstRead := make(chan struct{})
	done := make(chan struct{})
	go func() {
		defer close(done)
		input := make([]byte, chunkSize)
		for op := 0; op < operations; op++ {
			for i := range input {
				input[i] = byte((op*chunkSize + i) % 251)
			}
			replay.append(input)
			if op == 0 {
				close(firstAppended)
				<-firstRead
			}
			if op%16 == 0 {
				runtime.Gosched()
			}
		}
		finished.Store(true)
	}()
	<-firstAppended
	previous := int64(0)
	var failure error
	check := func() {
		got, total := replay.snapshot()
		if total < previous || total < 0 || total > finalTotal || total%chunkSize != 0 ||
			int64(len(got)) != min(total, int64(ptyReplayBufferLimit)) {
			failure = errors.New("inconsistent snapshot length/total")
			return
		}
		previous = total
		start := total - int64(len(got))
		for i, b := range got {
			if b != byte((start+int64(i))%251) {
				failure = errors.New("snapshot bytes do not match the stream offset")
				return
			}
		}
	}
	check() // The writer waits here, so a partial-stream snapshot is guaranteed.
	close(firstRead)
	for !finished.Load() {
		check()
		runtime.Gosched()
	}
	<-done
	check()
	if previous != finalTotal {
		return errors.New("writer did not complete its expected byte count")
	}
	return failure
}

func checkedMultiply(a, b uint64) (uint64, error) {
	if b != 0 && a > maxCount/b {
		return 0, errors.New("work count overflows int64")
	}
	return a * b, nil
}

// This oracle indexes the mathematical concatenation prefill || repeated work.
// It neither calls append nor duplicates the buffer's trim/grow algorithm.
func expectedSnapshot(prefill, work []byte, workBytes uint64) []byte {
	total := uint64(len(prefill)) + workBytes
	retained := min(total, uint64(ptyReplayBufferLimit))
	want := make([]byte, int(retained))
	for i := range want {
		position := total - retained + uint64(i)
		if position < uint64(len(prefill)) {
			want[i] = prefill[int(position)]
		} else {
			want[i] = work[int((position-uint64(len(prefill)))%uint64(len(work)))]
		}
	}
	return want
}

func measure(opts options, in *bufio.Reader, out *bufio.Writer) error {
	f, err := loadFixture(opts.fixtureDir, "mixed.bin", "chunks.csv", false)
	if err != nil {
		return err
	}
	prefill, err := os.ReadFile(filepath.Join(opts.fixtureDir, "prefill.bin"))
	if err != nil {
		return err
	}
	if len(prefill) != ptyReplayBufferLimit || len(f.payload) == 0 {
		return errors.New("prefill must be exactly 2 MiB and mixed payload must be nonempty")
	}
	var expectedBytes, expectedOps uint64
	if opts.mode == "paced" {
		expectedOps = pacedOperations
		for i := 0; i < pacedOperations; i++ {
			n := uint64(len(f.chunks[i%len(f.chunks)]))
			if expectedBytes > maxCount-n {
				return errors.New("paced byte count overflows int64")
			}
			expectedBytes += n
		}
	} else {
		expectedOps, err = checkedMultiply(opts.repetitions, uint64(len(f.chunks)))
		if err != nil {
			return err
		}
		expectedBytes, err = checkedMultiply(opts.repetitions, uint64(len(f.payload)))
		if err != nil {
			return err
		}
	}
	if expectedBytes > maxCount-uint64(len(prefill)) {
		return errors.New("prefill plus work overflows int64")
	}
	want := expectedSnapshot(prefill, f.payload, expectedBytes)
	if err := emit(out, map[string]string{"event": "READY", "language": "go", "mode": opts.mode}); err != nil {
		return err
	}
	if err := command(in, "WARMUP"); err != nil {
		return err
	}
	if err := warmup(f); err != nil {
		return err
	}
	var replay ptyReplayBuffer
	replay.append(prefill)
	if err := emit(out, map[string]string{"event": "WARMED", "language": "go", "mode": opts.mode}); err != nil {
		return err
	}
	if err := command(in, "START"); err != nil {
		return err
	}
	var completedBytes, completedOps uint64
	var maxLateness time.Duration
	start := time.Now()
	if opts.mode == "paced" {
		for i := 0; i < pacedOperations; i++ {
			deadline := start.Add(time.Duration(i) * pacedInterval)
			sleepUntil(deadline)
			if late := time.Since(deadline); late > maxLateness {
				maxLateness = late
			}
			chunk := f.chunks[i%len(f.chunks)]
			replay.append(chunk)
			completedBytes += uint64(len(chunk))
			completedOps++
		}
		sleepUntil(start.Add(pacedOperations * pacedInterval))
	} else {
		for repetition := uint64(0); repetition < opts.repetitions; repetition++ {
			for _, chunk := range f.chunks {
				replay.append(chunk)
				completedBytes += uint64(len(chunk))
				completedOps++
			}
		}
	}
	snapshotStart := time.Now()
	got, total := replay.snapshot()
	stop := time.Now()
	record := doneRecord{
		Event: "MEASURED", Language: "go", Mode: opts.mode,
		WallMS: milliseconds(stop.Sub(start)), SnapshotMS: milliseconds(stop.Sub(snapshotStart)),
		ExpectedBytes: expectedBytes, CompletedBytes: completedBytes,
		ExpectedOps: expectedOps, CompletedOps: completedOps, Total: total,
		MaxLatenessMS: milliseconds(maxLateness),
	}
	if err := emit(out, record); err != nil {
		return err
	}
	if err := command(in, "CHECK"); err != nil {
		return err
	}
	if completedBytes != expectedBytes || completedOps != expectedOps ||
		total != int64(uint64(len(prefill))+expectedBytes) || !bytes.Equal(got, want) {
		return errors.New("measured output or completed work differs from independent oracle")
	}
	record.Event = "DONE"
	record.OutputHash = fnv1a64(got)
	record.OK = true
	if err := emit(out, record); err != nil {
		return err
	}
	err = command(in, "EXIT")
	// Retain replay, snapshot, fixtures, and oracle through EXIT in both languages.
	runtime.KeepAlive(&replay)
	runtime.KeepAlive(got)
	runtime.KeepAlive(prefill)
	runtime.KeepAlive(want)
	runtime.KeepAlive(f)
	return err
}

func warmup(f fixture) error {
	var replay ptyReplayBuffer
	var total uint64
	start := time.Now()
	for {
		if total > maxCount-uint64(len(f.payload)) {
			return errors.New("warmup total overflows int64")
		}
		for _, chunk := range f.chunks {
			replay.append(chunk)
		}
		total += uint64(len(f.payload))
		if time.Since(start) >= warmupDuration {
			return nil
		}
	}
}

func sleepUntil(deadline time.Time) {
	for {
		delay := time.Until(deadline)
		if delay <= 0 {
			return
		}
		time.Sleep(delay)
	}
}

func command(in *bufio.Reader, expected string) error {
	// Limit malformed control input without allocating an unbounded line.
	line, err := in.ReadSlice('\n')
	if err != nil {
		if errors.Is(err, io.EOF) {
			return fmt.Errorf("stdin closed before %s newline", expected)
		}
		return fmt.Errorf("read %s: %w", expected, err)
	}
	line = bytes.TrimSuffix(line, []byte{'\n'})
	line = bytes.TrimSuffix(line, []byte{'\r'})
	if string(line) != expected {
		return fmt.Errorf("expected command %s", expected)
	}
	return nil
}

func emit(out *bufio.Writer, value any) error {
	if err := json.NewEncoder(out).Encode(value); err != nil {
		return err
	}
	return out.Flush()
}

func milliseconds(d time.Duration) float64 {
	return float64(d) / float64(time.Millisecond)
}

func fnv1a64(data []byte) string {
	hash := uint64(14695981039346656037)
	for _, b := range data {
		hash ^= uint64(b)
		hash *= 1099511628211
	}
	return fmt.Sprintf("%016x", hash)
}
