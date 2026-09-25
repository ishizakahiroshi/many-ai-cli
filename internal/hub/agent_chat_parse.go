package hub

import (
	"bytes"
	"container/list"
	"encoding/json"
	"fmt"
	"io"
	"os"
	"strings"
	"time"
	"unicode/utf8"

	"many-ai-cli/internal/sessionlog"
)

const (
	agentChatLineMax             = 8 * 1024 * 1024
	agentChatReadBuffer          = 64 * 1024
	agentChatDefaultMessageMax   = 200
	agentChatLiveMessageMax      = 200
	agentChatBatchBytesMax       = 8 * 1024 * 1024
	agentChatTextMax             = 64 * 1024
	agentChatThinkingMax         = 64
	agentChatToolsMax            = 64
	agentChatPendingToolMax      = 256
	agentChatPendingBytesMax     = 4 * 1024 * 1024
	agentChatReadBytesMax        = 4 * 1024 * 1024
	agentChatReadRecordsMax      = 256
	agentChatPageBytesMax        = 16 * 1024 * 1024
	agentChatPageRecordsMax      = 512
	agentChatReadTimeBudget      = 100 * time.Millisecond
	agentChatLegacyTailBytesMax  = agentChatLineMax + agentChatReadBuffer
	agentChatLegacyTailRecordMax = agentChatPageRecordsMax
)

// agentChatMessage is the provider-neutral representation sent to the browser.
// It is deliberately smaller than either provider's native event schema.
type agentChatMessage struct {
	Role        string          `json:"role"`
	Kind        string          `json:"kind,omitempty"`
	Text        string          `json:"text,omitempty"`
	Thinking    []string        `json:"thinking,omitempty"`
	Tools       []agentChatTool `json:"tools,omitempty"`
	TS          string          `json:"ts,omitempty"`
	MessageID   string          `json:"message_id,omitempty"`
	sourceStart int64
	sourceEnd   int64
}

type agentChatTool struct {
	ID     string `json:"id,omitempty"`
	Name   string `json:"name"`
	Input  string `json:"input,omitempty"`
	Result string `json:"result,omitempty"`
}

type agentChatLineHandler func([]byte) error

type agentChatReadBudget struct {
	MaxBytes   int64
	MaxRecords int
	Deadline   time.Time
	Clock      func() time.Time
}

type agentChatReadStats struct {
	// Offset is the next physical byte to read. It may point inside a line
	// when continuation state is retained by the live poller.
	Offset int64
	// SafeOffset is the last newline-aligned boundary whose record has been
	// consumed. Tail-page callers use this for live prime cursor placement.
	SafeOffset     int64
	SnapshotEnd    int64
	BytesRead      int64
	Records        int
	DecodedRecords int
	HitBudget      bool
	Complete       bool
	// TailPageReady is true only when the bounded tail page was fully read and
	// its record selection finished without a deadline interruption.
	TailPageReady bool
	// DecodeCommitted is true only when every selected tail record was decoded.
	// Live prime may use SafeOffset only after this commit boundary.
	DecodeCommitted bool
	Elapsed         time.Duration
}

func agentChatBudgetNow(budget agentChatReadBudget) time.Time {
	if budget.Clock != nil {
		return budget.Clock()
	}
	return time.Now()
}

func agentChatBudgetExpired(budget agentChatReadBudget) bool {
	return !agentChatBudgetNow(budget).Before(budget.Deadline)
}

// agentChatReadState is owned by one live parser. record is retained only up
// to agentChatLineMax; an oversized line is represented by oversized=true and
// is discarded while its physical cursor continues to advance to the newline.
// Keeping the cursor and this state together prevents a >read-budget line from
// being reread from its first byte on every poll.
type agentChatReadState struct {
	nextOffset         int64
	lastCompleteOffset int64
	lineBytes          int64
	record             []byte
	oversized          bool
}

func (state *agentChatReadState) reset(offset int64) {
	state.nextOffset = offset
	state.lastCompleteOffset = offset
	state.lineBytes = 0
	state.record = nil
	state.oversized = false
}

// readAgentChatRange consumes a strictly bounded byte window while retaining
// partial-record state between calls. Direct file reads make BytesRead an
// actual I/O cap rather than a logical buffered-read count.
func readAgentChatRange(path string, offset int64, continuation *agentChatReadState, budget agentChatReadBudget, handle agentChatLineHandler) (stats agentChatReadStats, err error) {
	started := time.Now()
	defer func() { stats.Elapsed = time.Since(started) }()
	if offset < 0 {
		offset = 0
	}
	if budget.MaxBytes <= 0 {
		budget.MaxBytes = agentChatReadBytesMax
	}
	if budget.MaxRecords <= 0 {
		budget.MaxRecords = agentChatReadRecordsMax
	}
	if budget.Deadline.IsZero() {
		budget.Deadline = agentChatBudgetNow(budget).Add(agentChatReadTimeBudget)
	}
	if continuation == nil {
		continuation = &agentChatReadState{}
	}
	if continuation.nextOffset != offset {
		// A caller may intentionally start a fresh parse at an arbitrary byte
		// boundary. Production live polls pass the previous physical cursor,
		// so this branch does not discard an in-flight line during normal use.
		continuation.reset(offset)
	}
	stats.Offset = continuation.nextOffset
	stats.SafeOffset = continuation.lastCompleteOffset

	info, err := os.Stat(path)
	if err != nil {
		return stats, err
	}
	if info.IsDir() {
		return stats, fmt.Errorf("transcript path is a directory")
	}
	if info.Size() < offset {
		stats.Complete = true
		return stats, nil
	}
	f, err := os.Open(path)
	if err != nil {
		return stats, err
	}
	defer f.Close()
	if _, err := f.Seek(offset, io.SeekStart); err != nil {
		return stats, err
	}

	buf := make([]byte, agentChatReadBuffer)
	for stats.BytesRead < budget.MaxBytes && stats.Records < budget.MaxRecords {
		if agentChatBudgetExpired(budget) {
			stats.HitBudget = true
			break
		}
		remaining := budget.MaxBytes - stats.BytesRead
		readBuf := buf
		if remaining < int64(len(readBuf)) {
			readBuf = readBuf[:remaining]
		}
		if len(readBuf) == 0 {
			stats.HitBudget = true
			break
		}
		n, readErr := f.Read(readBuf)
		stats.BytesRead += int64(n)
		for pos := 0; pos < n; {
			if agentChatBudgetExpired(budget) {
				stats.HitBudget = true
				break
			}
			lineEnd := bytes.IndexByte(readBuf[pos:n], '\n')
			if lineEnd < 0 {
				fragment := readBuf[pos:n]
				if err := consumeAgentChatFragment(continuation, fragment, false, &stats, handle); err != nil {
					return stats, err
				}
				break
			}
			fragment := readBuf[pos : pos+lineEnd+1]
			if err := consumeAgentChatFragment(continuation, fragment, true, &stats, handle); err != nil {
				return stats, err
			}
			pos += lineEnd + 1
			if stats.Records >= budget.MaxRecords {
				stats.HitBudget = true
				break
			}
		}
		if stats.Records >= budget.MaxRecords || stats.BytesRead >= budget.MaxBytes || agentChatBudgetExpired(budget) {
			stats.HitBudget = true
			break
		}
		if readErr == io.EOF {
			stats.Complete = continuation.lineBytes == 0
			break
		}
		if readErr != nil {
			return stats, readErr
		}
		if n == 0 {
			break
		}
	}
	if stats.BytesRead >= budget.MaxBytes || stats.Records >= budget.MaxRecords || agentChatBudgetExpired(budget) {
		stats.HitBudget = true
	}
	stats.Offset = continuation.nextOffset
	stats.SafeOffset = continuation.lastCompleteOffset
	if continuation.lineBytes == 0 && stats.Offset >= info.Size() {
		stats.Complete = true
	}
	return stats, nil
}

func consumeAgentChatFragment(continuation *agentChatReadState, fragment []byte, complete bool, stats *agentChatReadStats, handle agentChatLineHandler) error {
	if continuation == nil || stats == nil || len(fragment) == 0 {
		return nil
	}
	continuation.lineBytes += int64(len(fragment))
	continuation.nextOffset += int64(len(fragment))
	if !continuation.oversized && continuation.lineBytes <= agentChatLineMax {
		continuation.record = append(continuation.record, fragment...)
	} else {
		continuation.oversized = true
		continuation.record = nil
	}
	if !complete {
		stats.Offset = continuation.nextOffset
		return nil
	}

	stats.Offset = continuation.nextOffset
	stats.SafeOffset = continuation.nextOffset
	stats.Records++
	if !continuation.oversized && len(continuation.record) > 0 && handle != nil {
		if err := handle(continuation.record[:len(continuation.record)-1]); err != nil {
			return err
		}
	}
	continuation.lineBytes = 0
	continuation.record = nil
	continuation.oversized = false
	continuation.lastCompleteOffset = continuation.nextOffset
	return nil
}

// readAgentChatTail reads complete records with a bounded compatibility
// budget. Production API requests use the page/cursor readers below; this
// helper remains for focused parser tests and older callers.
func readAgentChatTail(path string, offset int64, handle agentChatLineHandler) (int64, error) {
	continuation := &agentChatReadState{}
	stats, err := readAgentChatRange(path, offset, continuation, agentChatReadBudget{
		MaxBytes: agentChatLegacyTailBytesMax, MaxRecords: agentChatLegacyTailRecordMax,
		Deadline: time.Now().Add(5 * time.Second),
	}, handle)
	return stats.Offset, err
}

type claudeTranscriptLine struct {
	Type        string `json:"type"`
	SessionID   string `json:"sessionId"`
	Timestamp   string `json:"timestamp"`
	IsSidechain bool   `json:"isSidechain"`
	Message     struct {
		Role    string          `json:"role"`
		Content json.RawMessage `json:"content"`
	} `json:"message"`
}

// anthropicContentBlock is one element of an Anthropic-style content array
// (text / thinking / tool_use / tool_result). Claude Code writes it, and so does
// Command Code — measured 2026-09-18 over 20 transcripts / 212 records, see
// docs/local/reference/reference_transcript-sources.md. The two CLIs' envelopes
// differ (record type names, extra fields), the block level does not, so the
// block-level parsing below is shared instead of copied per provider.
type anthropicContentBlock struct {
	Type      string          `json:"type"`
	Text      string          `json:"text"`
	Thinking  string          `json:"thinking"`
	ID        string          `json:"id"`
	Name      string          `json:"name"`
	Input     json.RawMessage `json:"input"`
	ToolUseID string          `json:"tool_use_id"`
	Content   json.RawMessage `json:"content"`
}

type agentChatParseState struct {
	messages            []*agentChatMessage
	tools               map[string]*agentChatToolRef
	pendingOrder        *list.List
	pendingElems        map[string]*list.Element
	pendingMessageRefs  map[*agentChatMessage]int
	pendingMessageBytes map[*agentChatMessage]int
	pendingBytes        int
	maxMessages         int
	maxBytes            int
	parsedMessages      int
	batchBytes          int
	batch               uint64
	pageModeTail        bool
	currentRecordStart  int64
	currentRecordEnd    int64
	readState           agentChatReadState
	lastRead            agentChatReadStats
	codexCompletions    []codexTaskCompletion
}

type agentChatToolRef struct {
	message      *agentChatMessage
	index        int
	batch        uint64
	messageBytes int
}

func newAgentChatParseState() *agentChatParseState {
	return newAgentChatParseStateWithPage(agentChatLiveMessageMax, agentChatBatchBytesMax, -1)
}

func newAgentChatParseStateWithPage(maxMessages, maxBytes, offset int) *agentChatParseState {
	if maxMessages <= 0 {
		maxMessages = agentChatDefaultMessageMax
	}
	if maxBytes <= 0 {
		maxBytes = agentChatBatchBytesMax
	}
	return &agentChatParseState{
		maxMessages:         maxMessages,
		maxBytes:            maxBytes,
		tools:               make(map[string]*agentChatToolRef),
		pendingOrder:        list.New(),
		pendingElems:        make(map[string]*list.Element),
		pendingMessageRefs:  make(map[*agentChatMessage]int),
		pendingMessageBytes: make(map[*agentChatMessage]int),
		pageModeTail:        offset < 0,
	}
}

func (state *agentChatParseState) beginBatch() {
	if state == nil {
		return
	}
	state.messages = nil
	state.codexCompletions = nil
	state.batchBytes = 0
	state.batch++
	if state.batch == 0 {
		state.batch = 1
	}
	for id, ref := range state.tools {
		if ref == nil || ref.message == nil {
			state.removePendingTool(id)
		}
	}
}

func agentChatMessageBytes(message *agentChatMessage) int {
	if message == nil {
		return 0
	}
	n := len(message.Role) + len(message.Kind) + len(message.Text) + len(message.TS) + len(message.MessageID)
	for _, item := range message.Thinking {
		n += len(item)
	}
	for _, tool := range message.Tools {
		n += len(tool.ID) + len(tool.Name) + len(tool.Input) + len(tool.Result)
	}
	return n
}

func (state *agentChatParseState) evictOldestMessage() {
	if len(state.messages) == 0 {
		return
	}
	old := state.messages[0]
	state.messages = state.messages[1:]
	state.batchBytes -= agentChatMessageBytes(old)
	if state.batchBytes < 0 {
		state.batchBytes = 0
	}
	for _, tool := range old.Tools {
		state.removePendingTool(tool.ID)
	}
}

func (state *agentChatParseState) appendMessage(message *agentChatMessage, count bool) {
	if state == nil || message == nil {
		return
	}
	boundAgentChatMessage(message)
	if message.sourceEnd == 0 && state.currentRecordEnd > 0 {
		message.sourceStart = state.currentRecordStart
		message.sourceEnd = state.currentRecordEnd
	}
	if count {
		state.parsedMessages++
	}
	messageBytes := agentChatMessageBytes(message)
	for len(state.messages) >= state.maxMessages || state.batchBytes+messageBytes > state.maxBytes {
		if len(state.messages) == 0 {
			break
		}
		state.evictOldestMessage()
	}
	state.messages = append(state.messages, message)
	state.batchBytes += messageBytes
}

func (state *agentChatParseState) outputMessages() []agentChatMessage {
	if state == nil || len(state.messages) == 0 {
		return nil
	}
	out := make([]agentChatMessage, 0, len(state.messages))
	for _, message := range state.messages {
		if message != nil {
			out = append(out, *message)
		}
	}
	return out
}

func registerAgentChatTools(state *agentChatParseState, message *agentChatMessage) {
	if state == nil || message == nil {
		return
	}
	if state.pendingOrder == nil {
		state.pendingOrder = list.New()
	}
	for index, tool := range message.Tools {
		if tool.ID == "" {
			continue
		}
		state.removePendingTool(tool.ID)
		messageBytes := agentChatMessageBytes(message)
		if messageBytes > agentChatPendingBytesMax {
			continue
		}
		alreadyCounted := state.pendingMessageRefs[message] > 0
		for len(state.tools) >= agentChatPendingToolMax || (!alreadyCounted && state.pendingBytes+messageBytes > agentChatPendingBytesMax) {
			if !state.evictOldestPendingTool() {
				break
			}
		}
		if len(state.tools) >= agentChatPendingToolMax || (!alreadyCounted && state.pendingBytes+messageBytes > agentChatPendingBytesMax) {
			continue
		}
		if !alreadyCounted {
			state.pendingMessageBytes[message] = messageBytes
			state.pendingBytes += messageBytes
		}
		ref := &agentChatToolRef{message: message, index: index, batch: state.batch, messageBytes: messageBytes}
		state.tools[tool.ID] = ref
		state.pendingMessageRefs[message]++
		state.pendingElems[tool.ID] = state.pendingOrder.PushBack(tool.ID)
	}
}

func (state *agentChatParseState) removePendingTool(id string) {
	if state == nil || id == "" {
		return
	}
	ref, ok := state.tools[id]
	if !ok {
		return
	}
	delete(state.tools, id)
	if elem := state.pendingElems[id]; elem != nil {
		state.pendingOrder.Remove(elem)
		delete(state.pendingElems, id)
	}
	if ref.message == nil {
		return
	}
	refs := state.pendingMessageRefs[ref.message] - 1
	if refs > 0 {
		state.pendingMessageRefs[ref.message] = refs
		return
	}
	delete(state.pendingMessageRefs, ref.message)
	if messageBytes := state.pendingMessageBytes[ref.message]; messageBytes > 0 {
		state.pendingBytes -= messageBytes
		if state.pendingBytes < 0 {
			state.pendingBytes = 0
		}
	}
	delete(state.pendingMessageBytes, ref.message)
}

func (state *agentChatParseState) evictOldestPendingTool() bool {
	if state == nil || state.pendingOrder == nil {
		return false
	}
	elem := state.pendingOrder.Front()
	if elem == nil {
		return false
	}
	state.removePendingTool(elem.Value.(string))
	return true
}

func (state *agentChatParseState) reaccountPendingMessage(message *agentChatMessage) {
	if state == nil || message == nil || state.pendingMessageRefs[message] == 0 {
		return
	}
	oldBytes := state.pendingMessageBytes[message]
	newBytes := agentChatMessageBytes(message)
	state.pendingMessageBytes[message] = newBytes
	state.pendingBytes += newBytes - oldBytes
	for state.pendingBytes > agentChatPendingBytesMax && len(state.tools) > 0 {
		if !state.evictOldestPendingTool() {
			break
		}
	}
}

func boundAgentChatMessage(message *agentChatMessage) {
	if message == nil {
		return
	}
	message.Text = limitAgentChatText(message.Text)
	if len(message.Thinking) > agentChatThinkingMax {
		message.Thinking = message.Thinking[:agentChatThinkingMax]
	}
	for i := range message.Thinking {
		message.Thinking[i] = limitAgentChatText(message.Thinking[i])
	}
	if len(message.Tools) > agentChatToolsMax {
		message.Tools = message.Tools[:agentChatToolsMax]
	}
	for i := range message.Tools {
		message.Tools[i].Name = limitAgentChatText(message.Tools[i].Name)
		message.Tools[i].Input = limitAgentChatText(message.Tools[i].Input)
		message.Tools[i].Result = limitAgentChatText(message.Tools[i].Result)
	}
	if message.MessageID == "" {
		message.MessageID = stableAgentChatMessageID(message)
	}
}

func stableAgentChatMessageID(message *agentChatMessage) string {
	if message == nil {
		return ""
	}
	for _, tool := range message.Tools {
		if tool.ID != "" {
			return "tool:" + tool.ID
		}
	}
	return ""
}

type agentChatTailRecord struct {
	start int64
	end   int64
	line  []byte
}

func readAgentChatTailPageWithBudget(path string, maxRecords int, endOffset int64, budget agentChatReadBudget) ([]agentChatTailRecord, agentChatReadStats, error) {
	started := time.Now()
	stats := agentChatReadStats{}
	if maxRecords <= 0 {
		maxRecords = agentChatPageRecordsMax
	}
	if maxRecords > agentChatPageRecordsMax {
		maxRecords = agentChatPageRecordsMax
	}
	if budget.MaxBytes <= 0 {
		budget.MaxBytes = agentChatPageBytesMax
	}
	if budget.MaxRecords <= 0 {
		budget.MaxRecords = agentChatPageRecordsMax
	}
	if budget.Deadline.IsZero() {
		budget.Deadline = agentChatBudgetNow(budget).Add(agentChatReadTimeBudget)
	}
	info, err := os.Stat(path)
	if err != nil {
		return nil, stats, err
	}
	if info.IsDir() {
		return nil, stats, fmt.Errorf("transcript path is a directory")
	}
	if endOffset < 0 || endOffset > info.Size() {
		endOffset = info.Size()
	}
	windowSize := endOffset
	if windowSize > int64(budget.MaxBytes) {
		windowSize = budget.MaxBytes
		stats.HitBudget = true
	}
	windowStart := endOffset - windowSize
	stats.SnapshotEnd = endOffset
	window := make([]byte, windowSize)
	f, err := os.Open(path)
	if err != nil {
		return nil, stats, err
	}
	defer f.Close()
	loadedStart := int(windowSize)
	for loadedStart > 0 {
		if agentChatBudgetExpired(budget) {
			stats.HitBudget = true
			break
		}
		chunkEnd := windowStart + int64(loadedStart)
		chunkStart := chunkEnd - int64(agentChatReadBuffer)
		if chunkStart < windowStart {
			chunkStart = windowStart
		}
		chunk := window[int(chunkStart-windowStart):int(chunkEnd-windowStart)]
		readN, readErr := f.ReadAt(chunk, chunkStart)
		stats.BytesRead += int64(readN)
		if readErr != nil && readErr != io.EOF {
			return nil, stats, readErr
		}
		if readN != len(chunk) {
			return nil, stats, io.ErrUnexpectedEOF
		}
		loadedStart = int(chunkStart - windowStart)
	}
	loadedWindow := window[loadedStart:]
	if len(loadedWindow) == 0 {
		stats.Offset = endOffset
		stats.SafeOffset = endOffset
		stats.SnapshotEnd = endOffset
		stats.TailPageReady = true
		stats.Complete = true
		stats.Elapsed = time.Since(started)
		return nil, stats, nil
	}
	if loadedStart > 0 {
		// The read budget expired before the snapshot window was assembled. The
		// zero-filled prefix is not input and must not be selected as records.
		stats.HitBudget = true
		stats.Offset = endOffset
		stats.Elapsed = time.Since(started)
		return nil, stats, nil
	}
	completeEnd := len(window)
	if completeEnd > loadedStart && window[completeEnd-1] != '\n' {
		lastNewline := bytes.LastIndexByte(loadedWindow, '\n')
		if lastNewline < 0 {
			if agentChatBudgetExpired(budget) {
				stats.HitBudget = true
				stats.Offset = endOffset
				stats.Elapsed = time.Since(started)
				return nil, stats, nil
			}
			stats.Offset = endOffset
			// A window with no newline and a non-zero start contains only a
			// continuation of an already oversized line. It is safe to skip
			// that line for live prime; a zero-start window must wait for the
			// line to complete.
			if windowStart > 0 {
				stats.SafeOffset = endOffset
			} else {
				stats.SafeOffset = 0
			}
			stats.TailPageReady = true
			stats.Elapsed = time.Since(started)
			return nil, stats, nil
		}
		completeEnd = loadedStart + lastNewline + 1
	}

	records := make([]agentChatTailRecord, 0, maxRecords)
	pos := completeEnd
	selectionReady := true
	for pos > loadedStart && len(records) < maxRecords {
		if agentChatBudgetExpired(budget) {
			stats.HitBudget = true
			stats.Offset = windowStart + int64(pos)
			selectionReady = false
			break
		}
		lineStart := bytes.LastIndexByte(window[loadedStart:pos-1], '\n')
		if lineStart < 0 {
			lineStart = loadedStart
		} else {
			lineStart = loadedStart + lineStart + 1
		}
		if lineStart == loadedStart && windowStart+int64(loadedStart) > 0 {
			// The window begins in the middle of a record. Do not parse that
			// partial record or expose a cursor that is not line aligned.
			break
		}
		records = append(records, agentChatTailRecord{
			start: windowStart + int64(lineStart),
			end:   windowStart + int64(pos),
			line:  append([]byte(nil), window[lineStart:pos-1]...),
		})
		pos = lineStart
	}
	if len(records) > 0 {
		if !stats.HitBudget {
			stats.Offset = records[len(records)-1].start
		}
	} else if !stats.HitBudget {
		stats.Offset = endOffset
	}
	stats.SafeOffset = windowStart + int64(completeEnd)
	stats.Records = len(records)
	stats.Complete = completeEnd == len(window) && loadedStart == 0
	stats.TailPageReady = selectionReady
	stats.HitBudget = stats.HitBudget || len(records) >= min(maxRecords, budget.MaxRecords)
	stats.Elapsed = time.Since(started)
	return records, stats, nil
}

func agentChatTailRecordLimit(maxMessages int) int {
	if maxMessages <= 0 {
		maxMessages = agentChatDefaultMessageMax
	}
	return min(maxMessages*2+16, agentChatPageRecordsMax)
}

func parseClaudeLine(state *agentChatParseState, line []byte) error {
	if len(line) > agentChatLineMax {
		return nil
	}
	var record claudeTranscriptLine
	if err := json.Unmarshal(line, &record); err != nil {
		return nil
	}
	parseClaudeRecord(state, record)
	return nil
}

func parseClaudeTranscript(path string, offset int64) ([]agentChatMessage, int64, error) {
	return parseClaudeTranscriptWithState(path, offset, newAgentChatParseState())
}

// agentChatRecordParser parses one JSONL record of a provider's transcript into
// state. Everything around it — the bounded reads, the budgets, the forward and
// backward cursors — is provider-neutral and lives in the three
// parseAgentChat* functions below, so adding a provider means adding its record
// parser, not another copy of the cursor logic.
type agentChatRecordParser func(state *agentChatParseState, line []byte) error

// agentChatRecordParserFor returns provider's record parser, or nil when the Hub
// has no parser for that provider's transcript.
//
// This is the dispatch half of provider_feature_source.go's StructuredTranscript
// column: the decision "is it worth opening this provider's transcript at all"
// is table-driven, the parsing of a given file format cannot be — the same split
// internal/subscription/usage_source.go makes with internal/usagelocal.
// TestAgentChatParsersMatchFeatureSourceTable holds the two in agreement in both
// directions.
func agentChatRecordParserFor(provider string) agentChatRecordParser {
	switch provider {
	case "claude":
		return parseClaudeLine
	case "codex":
		return parseCodexLine
	case "command-code":
		return parseCommandCodeLine
	}
	return nil
}

func parseAgentChatTranscriptWithState(path string, offset int64, state *agentChatParseState, parse agentChatRecordParser) ([]agentChatMessage, int64, error) {
	if state == nil {
		state = newAgentChatParseState()
	}
	state.beginBatch()
	decodedRecords := 0
	stats, err := readAgentChatRange(path, offset, &state.readState, agentChatReadBudget{
		MaxBytes: agentChatReadBytesMax, MaxRecords: agentChatReadRecordsMax,
		Deadline: time.Now().Add(agentChatReadTimeBudget),
	}, func(line []byte) error {
		decodedRecords++
		return parse(state, line)
	})
	stats.DecodedRecords = decodedRecords
	state.lastRead = stats
	return state.outputMessages(), stats.Offset, err
}

func parseAgentChatTailPage(path string, state *agentChatParseState, endOffset int64, parse agentChatRecordParser) ([]agentChatMessage, int64, error) {
	now := time.Now()
	return parseAgentChatTailPageWithBudget(path, state, endOffset, agentChatReadBudget{
		MaxBytes:   agentChatPageBytesMax,
		MaxRecords: agentChatPageRecordsMax,
		Deadline:   now.Add(agentChatReadTimeBudget),
	}, parse)
}

func parseAgentChatTailPageWithBudget(path string, state *agentChatParseState, endOffset int64, budget agentChatReadBudget, parse agentChatRecordParser) ([]agentChatMessage, int64, error) {
	if state == nil {
		state = newAgentChatParseState()
	}
	state.beginBatch()
	if budget.Deadline.IsZero() {
		budget.Deadline = agentChatBudgetNow(budget).Add(agentChatReadTimeBudget)
	}
	records, stats, err := readAgentChatTailPageWithBudget(path, agentChatTailRecordLimit(state.maxMessages), endOffset, budget)
	if err != nil {
		return nil, stats.Offset, err
	}
	nextCursor := stats.Offset
	if !stats.TailPageReady {
		// The snapshot was not selected atomically. Keep the API cursor at the
		// snapshot boundary, while live prime ignores it and retries the page.
		stats.HitBudget = true
		nextCursor = stats.SnapshotEnd
		stats.Offset = nextCursor
		state.lastRead = stats
		return nil, stats.Offset, nil
	}
	// Once a bounded page has been selected, commit the whole page or retry
	// the same snapshot. Returning records parsed from only the old side of a
	// page would move a backward cursor past newer, not-yet-parsed records.
	if len(records) > 0 && agentChatBudgetExpired(budget) {
		stats.HitBudget = true
		nextCursor = stats.SnapshotEnd
	} else {
		for index := len(records) - 1; index >= 0; index-- {
			state.currentRecordStart = records[index].start
			state.currentRecordEnd = records[index].end
			if err := parse(state, records[index].line); err != nil {
				state.currentRecordStart = 0
				state.currentRecordEnd = 0
				return state.outputMessages(), stats.Offset, err
			}
			stats.DecodedRecords++
			nextCursor = records[index].start
			// Do not break here. The page's selected record range is bounded,
			// and committing only a prefix would make the backward cursor skip
			// the newer records that remain in this same page.
			if agentChatBudgetExpired(budget) {
				stats.HitBudget = true
			}
		}
	}
	state.currentRecordStart = 0
	state.currentRecordEnd = 0
	if len(state.messages) > 0 {
		messageCursor := state.messages[0].sourceStart
		if !stats.HitBudget {
			nextCursor = messageCursor
		} else if nextCursor == 0 || messageCursor < nextCursor {
			nextCursor = messageCursor
		}
	}
	stats.DecodeCommitted = stats.TailPageReady &&
		stats.DecodedRecords == len(records) &&
		(len(records) > 0 || !stats.HitBudget)
	stats.Offset = nextCursor
	state.lastRead = stats
	return state.outputMessages(), stats.Offset, nil
}

func parseClaudeTranscriptWithState(path string, offset int64, state *agentChatParseState) ([]agentChatMessage, int64, error) {
	return parseAgentChatTranscriptWithState(path, offset, state, parseClaudeLine)
}

func parseClaudeTranscriptTailPage(path string, state *agentChatParseState, endOffset int64) ([]agentChatMessage, int64, error) {
	return parseAgentChatTailPage(path, state, endOffset, parseClaudeLine)
}

func parseClaudeTranscriptTailPageWithBudget(path string, state *agentChatParseState, endOffset int64, budget agentChatReadBudget) ([]agentChatMessage, int64, error) {
	return parseAgentChatTailPageWithBudget(path, state, endOffset, budget, parseClaudeLine)
}

func parseClaudeRecord(state *agentChatParseState, record claudeTranscriptLine) {
	if state == nil || record.Type != "user" && record.Type != "assistant" {
		return
	}
	blocks, ok := anthropicContentBlocks(record.Message.Content)
	if !ok {
		return
	}
	if record.Message.Role == "" {
		record.Message.Role = record.Type
	}
	if record.Message.Role == "user" {
		appendAnthropicUserMessage(state, blocks, record.Timestamp)
		return
	}
	if record.Message.Role == "assistant" {
		appendAnthropicAssistantMessage(state, blocks, record.Timestamp, record.IsSidechain)
	}
}

// anthropicContentBlocks decodes a message's content field, accepting both the
// block array and the plain-string shape the same CLIs also emit.
func anthropicContentBlocks(raw json.RawMessage) ([]anthropicContentBlock, bool) {
	var blocks []anthropicContentBlock
	if err := json.Unmarshal(raw, &blocks); err == nil {
		return blocks, true
	}
	var text string
	if json.Unmarshal(raw, &text) == nil && strings.TrimSpace(text) != "" {
		return []anthropicContentBlock{{Type: "text", Text: text}}, true
	}
	return nil, false
}

func appendAnthropicUserMessage(state *agentChatParseState, blocks []anthropicContentBlock, ts string) {
	var textParts []string
	for _, block := range blocks {
		switch block.Type {
		case "text":
			if strings.TrimSpace(block.Text) != "" {
				textParts = append(textParts, block.Text)
			}
		case "tool_result":
			attachAnthropicToolResult(state, block)
		}
	}
	text := strings.TrimSpace(strings.Join(textParts, "\n"))
	if text == "" || isSyntheticAgentUserText(text) {
		return
	}
	state.appendMessage(&agentChatMessage{
		Role: "user",
		Kind: "text",
		Text: maskAgentChatText(text),
		TS:   ts,
	}, true)
}

func appendAnthropicAssistantMessage(state *agentChatParseState, blocks []anthropicContentBlock, ts string, isSidechain bool) {
	var textParts []string
	var thinking []string
	var tools []agentChatTool
	for _, block := range blocks {
		switch block.Type {
		case "text":
			if strings.TrimSpace(block.Text) != "" {
				textParts = append(textParts, block.Text)
			}
		case "thinking":
			if strings.TrimSpace(block.Thinking) != "" {
				thinking = append(thinking, maskAgentChatText(block.Thinking))
			}
		case "tool_use":
			tool := agentChatTool{
				ID:    block.ID,
				Name:  maskAgentChatText(block.Name),
				Input: summarizeAgentChatJSON(block.Input),
			}
			tools = append(tools, tool)
		}
	}
	text := strings.TrimSpace(strings.Join(textParts, "\n"))
	if isSidechain {
		if text != "" {
			thinking = append(thinking, maskAgentChatText(text))
		}
		text = ""
	}
	if text == "" && len(thinking) == 0 && len(tools) == 0 {
		return
	}
	kind := "text"
	if isSidechain {
		kind = "sidechain"
	} else if text == "" && len(tools) > 0 {
		kind = "tool"
	} else if text == "" {
		kind = "thinking"
	}
	message := &agentChatMessage{
		Role:     "assistant",
		Kind:     kind,
		Text:     maskAgentChatText(text),
		Thinking: thinking,
		Tools:    tools,
		TS:       ts,
	}
	state.appendMessage(message, true)
	registerAgentChatTools(state, message)
}

func attachAnthropicToolResult(state *agentChatParseState, block anthropicContentBlock) {
	if state == nil {
		return
	}
	toolID := block.ToolUseID
	if toolID == "" {
		return
	}
	ref, ok := state.tools[toolID]
	if !ok || ref == nil || ref.message == nil || ref.index < 0 || ref.index >= len(ref.message.Tools) {
		return
	}
	result := extractAgentChatText(block.Content)
	if result == "" {
		result = block.Text
	}
	if result == "" {
		return
	}
	ref.message.Tools[ref.index].Result = maskAgentChatText(result)
	state.reaccountPendingMessage(ref.message)
	if ref.batch != state.batch {
		ref.batch = state.batch
		state.appendMessage(ref.message, false)
	}
	state.removePendingTool(toolID)
}

// isSyntheticAgentUserText drops the injected user turns both CLIs write into
// their transcripts (slash-command echoes, system reminders). Command Code uses
// the same Anthropic-style markers; if a future version stops emitting them the
// check simply never matches.
func isSyntheticAgentUserText(text string) bool {
	return strings.Contains(text, "<command-name>") || strings.Contains(text, "<system-reminder>")
}

// commandCodeTranscriptLine is one record of
// ~/.commandcode/projects/<slug>/<session id>.jsonl.
//
// Measured 2026-09-18 over 20 transcripts / 212 records on this machine
// (docs/local/reference/reference_transcript-sources.md). Record types seen:
// "message" (174), "session" (10), "model_change" (5), plus 23 index records
// that carry no type at all (turnNumber / messageCount / files / prompt). Only
// "message" holds conversation, and its message object is Anthropic-shaped, so
// the block level is parsed by the shared anthropic* helpers above rather than
// by a second copy.
//
// The other record types are ignored on purpose: "session" only repeats cwd and
// a version, "model_change" duplicates what the session already reports through
// the wrapper, and the index records are a turn table for Command Code's own UI.
// Nothing here reads the file's usage/cost fields — remaining quota has its own
// table (internal/subscription/usage_source.go) and Command Code is not in it.
type commandCodeTranscriptLine struct {
	Type      string `json:"type"`
	Timestamp string `json:"timestamp"`
	Message   struct {
		Role    string          `json:"role"`
		Content json.RawMessage `json:"content"`
	} `json:"message"`
}

func parseCommandCodeLine(state *agentChatParseState, line []byte) error {
	if len(line) > agentChatLineMax {
		return nil
	}
	var record commandCodeTranscriptLine
	if err := json.Unmarshal(line, &record); err != nil {
		return nil
	}
	parseCommandCodeRecord(state, record)
	return nil
}

func parseCommandCodeRecord(state *agentChatParseState, record commandCodeTranscriptLine) {
	if state == nil || record.Type != "message" {
		return
	}
	blocks, ok := anthropicContentBlocks(record.Message.Content)
	if !ok {
		return
	}
	switch record.Message.Role {
	case "user":
		// tool_result blocks arrive in user records, exactly as with Claude.
		appendAnthropicUserMessage(state, blocks, record.Timestamp)
	case "assistant":
		// Command Code has no sidechain concept in its transcript (no such field
		// was observed), so subagent output never has to be folded into thinking.
		appendAnthropicAssistantMessage(state, blocks, record.Timestamp, false)
	}
}

func parseCommandCodeTranscriptWithState(path string, offset int64, state *agentChatParseState) ([]agentChatMessage, int64, error) {
	return parseAgentChatTranscriptWithState(path, offset, state, parseCommandCodeLine)
}

func parseCommandCodeTranscriptTailPage(path string, state *agentChatParseState, endOffset int64) ([]agentChatMessage, int64, error) {
	return parseAgentChatTailPage(path, state, endOffset, parseCommandCodeLine)
}

type codexRolloutLine struct {
	Type      string          `json:"type"`
	Timestamp string          `json:"timestamp"`
	Payload   json.RawMessage `json:"payload"`
}

type codexPayload struct {
	Type                                   string                            `json:"type"`
	Role                                   string                            `json:"role"`
	Text                                   string                            `json:"text"`
	Message                                string                            `json:"message"`
	Name                                   string                            `json:"name"`
	CallID                                 string                            `json:"call_id"`
	Arguments                              json.RawMessage                   `json:"arguments"`
	Input                                  json.RawMessage                   `json:"input"`
	Output                                 json.RawMessage                   `json:"output"`
	Content                                json.RawMessage                   `json:"content"`
	Summary                                json.RawMessage                   `json:"summary"`
	TurnID                                 string                            `json:"turn_id"`
	CompletedAt                            float64                           `json:"completed_at"`
	LastAgentMessage                       json.RawMessage                   `json:"last_agent_message"`
	InternalChatMessageMetadataPassthrough *codexInternalChatMessageMetadata `json:"internal_chat_message_metadata_passthrough"`
}

type codexInternalChatMessageMetadata struct {
	ContentItemKinds []string `json:"content_item_kinds"`
}

type codexTaskCompletion struct {
	TurnID           string
	At               string
	LastAgentMessage string
}

func parseCodexRollout(path string, offset int64) ([]agentChatMessage, int64, error) {
	return parseCodexRolloutWithState(path, offset, newAgentChatParseState())
}

func parseCodexLine(state *agentChatParseState, line []byte) error {
	if len(line) > agentChatLineMax {
		return nil
	}
	var record codexRolloutLine
	if err := json.Unmarshal(line, &record); err != nil {
		return nil
	}
	parseCodexRecord(state, record)
	return nil
}

func parseCodexRolloutWithState(path string, offset int64, state *agentChatParseState) ([]agentChatMessage, int64, error) {
	return parseAgentChatTranscriptWithState(path, offset, state, parseCodexLine)
}

func parseCodexRolloutTailPage(path string, state *agentChatParseState, endOffset int64) ([]agentChatMessage, int64, error) {
	return parseAgentChatTailPage(path, state, endOffset, parseCodexLine)
}

func parseCodexRolloutTailPageWithBudget(path string, state *agentChatParseState, endOffset int64, budget agentChatReadBudget) ([]agentChatMessage, int64, error) {
	return parseAgentChatTailPageWithBudget(path, state, endOffset, budget, parseCodexLine)
}

func parseCodexRecord(state *agentChatParseState, record codexRolloutLine) {
	if state == nil || len(record.Payload) == 0 {
		return
	}
	var payload codexPayload
	if json.Unmarshal(record.Payload, &payload) != nil {
		return
	}
	ts := record.Timestamp
	switch record.Type {
	case "response_item":
		parseCodexResponseItem(state, payload, ts)
	case "event_msg":
		parseCodexEventMessage(state, payload, ts)
	}
}

func parseCodexResponseItem(state *agentChatParseState, payload codexPayload, ts string) {
	switch payload.Type {
	case "message":
		role := payload.Role
		if role != "user" && role != "assistant" {
			return
		}
		content := payload.Content
		if role == "user" {
			content = codexUserMessageContent(content, payload.InternalChatMessageMetadataPassthrough)
		}
		text, thinking, tools := parseCodexContent(content)
		if role == "user" && text == "" {
			return
		}
		if text == "" && len(thinking) == 0 && len(tools) == 0 {
			return
		}
		kind := "text"
		if role == "assistant" && text == "" && len(tools) > 0 {
			kind = "tool"
		} else if role == "assistant" && text == "" {
			kind = "thinking"
		}
		appendCodexMessage(state, agentChatMessage{
			Role:     role,
			Kind:     kind,
			Text:     maskAgentChatText(text),
			Thinking: thinking,
			Tools:    tools,
			TS:       ts,
		})
	case "reasoning":
		thinking := extractCodexSummary(payload.Summary)
		if len(thinking) == 0 {
			return
		}
		appendCodexMessage(state, agentChatMessage{Role: "assistant", Kind: "thinking", Thinking: thinking, TS: ts})
	case "function_call", "custom_tool_call":
		name := payload.Name
		input := payload.Arguments
		if len(input) == 0 {
			input = payload.Input
		}
		if name == "" && len(input) == 0 {
			return
		}
		appendCodexMessage(state, agentChatMessage{
			Role:  "assistant",
			Kind:  "tool",
			Tools: []agentChatTool{{ID: payload.CallID, Name: maskAgentChatText(name), Input: summarizeAgentChatJSON(input)}},
			TS:    ts,
		})
	case "function_call_output", "custom_tool_call_output":
		attachCodexToolResult(state, payload)
	}
}

// codexUserMessageContent removes Codex's injected AGENTS/environment context
// from user-role transcript messages. Recent Codex rollouts identify the
// origin of each content block in internal_chat_message_metadata_passthrough;
// only user-origin blocks belong in the sent-history view. Old rollouts do not
// carry this metadata, so retain their existing behavior.
func codexUserMessageContent(raw json.RawMessage, metadata *codexInternalChatMessageMetadata) json.RawMessage {
	if metadata == nil || len(metadata.ContentItemKinds) == 0 {
		return raw
	}

	var blocks []json.RawMessage
	if json.Unmarshal(raw, &blocks) != nil {
		if len(metadata.ContentItemKinds) == 1 && strings.HasPrefix(metadata.ContentItemKinds[0], "user.") {
			return raw
		}
		return nil
	}
	if len(blocks) != len(metadata.ContentItemKinds) {
		return nil
	}

	userBlocks := make([]json.RawMessage, 0, len(blocks))
	for i, kind := range metadata.ContentItemKinds {
		if strings.HasPrefix(kind, "user.") {
			userBlocks = append(userBlocks, blocks[i])
		}
	}
	if len(userBlocks) == 0 {
		return nil
	}
	filtered, err := json.Marshal(userBlocks)
	if err != nil {
		return nil
	}
	return filtered
}

func parseCodexEventMessage(state *agentChatParseState, payload codexPayload, ts string) {
	switch payload.Type {
	case "user_message":
		text := payload.Message
		if text == "" {
			text = payload.Text
		}
		if strings.TrimSpace(text) != "" {
			appendCodexMessage(state, agentChatMessage{Role: "user", Kind: "text", Text: maskAgentChatText(text), TS: ts})
		}
	case "agent_message":
		text := payload.Message
		if text == "" {
			text = payload.Text
		}
		if strings.TrimSpace(text) != "" {
			appendCodexMessage(state, agentChatMessage{Role: "assistant", Kind: "text", Text: maskAgentChatText(text), TS: ts})
		}
	case "agent_reasoning":
		text := payload.Message
		if text == "" {
			text = payload.Text
		}
		if strings.TrimSpace(text) != "" {
			appendCodexMessage(state, agentChatMessage{Role: "assistant", Kind: "thinking", Thinking: []string{maskAgentChatText(text)}, TS: ts})
		}
	case "task_complete":
		recordCodexTaskCompletion(state, payload, ts)
	}
}

func recordCodexTaskCompletion(state *agentChatParseState, payload codexPayload, ts string) {
	if state == nil {
		return
	}
	lastAgentMessage := ""
	if len(payload.LastAgentMessage) > 0 && string(payload.LastAgentMessage) != "null" {
		var text string
		if json.Unmarshal(payload.LastAgentMessage, &text) == nil {
			lastAgentMessage = maskAgentChatText(text)
		}
	}
	at := strings.TrimSpace(ts)
	if at == "" {
		at = codexCompletionTime(payload.CompletedAt)
	}
	if at == "" && strings.TrimSpace(payload.TurnID) == "" {
		return
	}
	state.codexCompletions = append(state.codexCompletions, codexTaskCompletion{
		TurnID:           strings.TrimSpace(payload.TurnID),
		At:               at,
		LastAgentMessage: lastAgentMessage,
	})
	const maxCodexCompletionsPerPoll = 16
	if len(state.codexCompletions) > maxCodexCompletionsPerPoll {
		state.codexCompletions = append([]codexTaskCompletion(nil), state.codexCompletions[len(state.codexCompletions)-maxCodexCompletionsPerPoll:]...)
	}
}

func (state *agentChatParseState) takeCodexCompletions() []codexTaskCompletion {
	if state == nil || len(state.codexCompletions) == 0 {
		return nil
	}
	out := append([]codexTaskCompletion(nil), state.codexCompletions...)
	state.codexCompletions = nil
	return out
}

func codexCompletionTime(seconds float64) string {
	if seconds <= 0 {
		return ""
	}
	if seconds >= 1e12 {
		seconds /= 1000
	}
	whole := int64(seconds)
	nanos := int64((seconds - float64(whole)) * float64(time.Second))
	return time.Unix(whole, nanos).UTC().Format(time.RFC3339)
}

func parseCodexContent(raw json.RawMessage) (string, []string, []agentChatTool) {
	var blocks []map[string]json.RawMessage
	if json.Unmarshal(raw, &blocks) != nil {
		text := extractAgentChatText(raw)
		return text, nil, nil
	}
	var textParts []string
	var thinking []string
	var tools []agentChatTool
	for _, block := range blocks {
		var kind string
		_ = json.Unmarshal(block["type"], &kind)
		switch kind {
		case "output_text", "input_text", "text":
			if text := extractAgentChatText(block["text"]); text != "" {
				textParts = append(textParts, text)
			} else if text := extractAgentChatText(block["content"]); text != "" {
				textParts = append(textParts, text)
			}
		case "summary_text", "reasoning", "reasoning_summary":
			if text := extractAgentChatText(block["text"]); text != "" {
				thinking = append(thinking, maskAgentChatText(text))
			}
		case "function_call", "custom_tool_call":
			var name, callID string
			_ = json.Unmarshal(block["name"], &name)
			_ = json.Unmarshal(block["call_id"], &callID)
			input := block["arguments"]
			if len(input) == 0 {
				input = block["input"]
			}
			tools = append(tools, agentChatTool{ID: callID, Name: maskAgentChatText(name), Input: summarizeAgentChatJSON(input)})
		}
	}
	return strings.TrimSpace(strings.Join(textParts, "\n")), thinking, tools
}

func appendCodexMessage(state *agentChatParseState, message agentChatMessage) {
	if state == nil {
		return
	}
	message.Text = maskAgentChatText(message.Text)
	for i := range message.Thinking {
		message.Thinking[i] = maskAgentChatText(message.Thinking[i])
	}
	for i := range message.Tools {
		message.Tools[i].Name = maskAgentChatText(message.Tools[i].Name)
		message.Tools[i].Input = maskAgentChatText(message.Tools[i].Input)
	}
	messagePtr := &message
	state.appendMessage(messagePtr, true)
	registerAgentChatTools(state, messagePtr)
}

func attachCodexToolResult(state *agentChatParseState, payload codexPayload) {
	if state == nil || payload.CallID == "" {
		return
	}
	ref, ok := state.tools[payload.CallID]
	if !ok || ref == nil || ref.message == nil || ref.index < 0 || ref.index >= len(ref.message.Tools) {
		return
	}
	result := extractAgentChatText(payload.Output)
	if result == "" {
		result = extractAgentChatText(payload.Content)
	}
	if result == "" {
		return
	}
	ref.message.Tools[ref.index].Result = maskAgentChatText(result)
	state.reaccountPendingMessage(ref.message)
	if ref.batch != state.batch {
		ref.batch = state.batch
		state.appendMessage(ref.message, false)
	}
	state.removePendingTool(payload.CallID)
}

func extractCodexSummary(raw json.RawMessage) []string {
	var items []map[string]json.RawMessage
	if json.Unmarshal(raw, &items) == nil {
		out := make([]string, 0, len(items))
		for _, item := range items {
			text := extractAgentChatText(item["text"])
			if text == "" {
				text = extractAgentChatText(item["summary_text"])
			}
			if text != "" {
				out = append(out, maskAgentChatText(text))
			}
		}
		return out
	}
	if text := extractAgentChatText(raw); text != "" {
		return []string{maskAgentChatText(text)}
	}
	return nil
}

func extractAgentChatText(raw json.RawMessage) string {
	if len(raw) == 0 || string(raw) == "null" {
		return ""
	}
	var text string
	if json.Unmarshal(raw, &text) == nil {
		return strings.TrimSpace(text)
	}
	var items []map[string]json.RawMessage
	if json.Unmarshal(raw, &items) == nil {
		var parts []string
		for _, item := range items {
			if value := extractAgentChatText(item["text"]); value != "" {
				parts = append(parts, value)
			} else if value := extractAgentChatText(item["content"]); value != "" {
				parts = append(parts, value)
			}
		}
		return strings.TrimSpace(strings.Join(parts, "\n"))
	}
	return ""
}

func summarizeAgentChatJSON(raw json.RawMessage) string {
	if len(raw) == 0 || string(raw) == "null" {
		return ""
	}
	if text := extractAgentChatText(raw); text != "" {
		return maskAgentChatText(text)
	}
	var value any
	if json.Unmarshal(raw, &value) != nil {
		return maskAgentChatText(string(raw))
	}
	data, err := json.Marshal(value)
	if err != nil {
		return ""
	}
	const maxSummary = 1200
	if len(data) > maxSummary {
		data = append(data[:maxSummary], "..."...)
	}
	return maskAgentChatText(string(data))
}

func maskAgentChatText(text string) string {
	return limitAgentChatText(sessionlog.MaskSecrets(strings.TrimSpace(text)))
}

func limitAgentChatText(text string) string {
	text = strings.TrimSpace(text)
	if len(text) <= agentChatTextMax {
		return text
	}
	cut := text[:agentChatTextMax]
	for len(cut) > 0 && !utf8.ValidString(cut) {
		cut = cut[:len(cut)-1]
	}
	return cut + "…"
}
