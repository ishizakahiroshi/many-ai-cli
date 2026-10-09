//go:build ignore

package main
import("encoding/json";"os";"regexp";"strconv";"strings";"unicode/utf8";"many-ai-cli/internal/proto")
var (
	workflowCSIRe = regexp.MustCompile(`\x1b\[[0-9;?]*[ -/]*[@-~]`)
	workflowOSCRe = regexp.MustCompile(`\x1b\][^\x07]*(?:\x07|\x1b\\)`)
	// `⎿`（U+23BF）と U+00A0 は Claude Code の Tip 行（案内文）の行頭記号（実測 67/67 件が
	// `⎿` + U+00A0 で始まる）。これが無いと Tip 行除外（workflowTipLineRe）が一致せず効かない
	// （敵対レビュー指摘 R8）。Go の \s は ASCII のみ（[\t\n\f\r ]）で U+00A0 を含まないため
	// 明示的に足す（TS 版の \s は U+00A0 を含むため見た目は非対称だが挙動は揃う）。
	workflowTreeRe    = regexp.MustCompile(`^[\s\x{00A0}│├└─╰╭╮╯┃┣┗┏┓┛┆┊▕▏▸▹‣•·⎿]+`)
	workflowHeaderRe  = regexp.MustCompile(`(?i)\bworkflows?\b`)
	workflowSummaryRe = regexp.MustCompile(`(?i)([0-9]{1,4})\s*/\s*([0-9]{1,4})\s+agents?\b`)
	workflowWaitingRe = regexp.MustCompile(`(?i)\bwaiting for\s+([0-9]{1,3})\s+dynamic\s+workflows?\s+to\s+finish\b`)
	workflowPercentRe = regexp.MustCompile(`([0-9]{1,3})\s*%`)
	workflowTimeRe    = regexp.MustCompile(`(?i)([0-9]+)\s*([hms])`)
	workflowMetricsRe = regexp.MustCompile(`\s{2,}`)
	// "Tip: …" は Claude Code の案内文で、"workflow" の語を含むことがあるが見出しではない。
	workflowTipLineRe = regexp.MustCompile(`(?i)^tip[:：]`)
	// 入力欄の区切り（罫線や ❯ プロンプト）。この行から下は常時表示の UI 領域で
	// Workflow の出力ではないため、見出し発見後の走査をここで打ち切る（誤検出防止）。
	// 罫線側は「行全体が横線文字（─━═）と空白だけ」の行に限定する（敵対レビュー指摘 R9）。
	// 以前は `[─━═]{3,}` を行のどこかに含むだけで打ち切っていたため、Workflow のブロック内の
	// 枠線（`╭─── Review ───╮` のように角や文字を含む行）まで区切りと誤認して本物のツリーを
	// 打ち切る恐れがあった。❯ プロンプト行は今までどおり行のどこかに含むだけで区切りとする
	// （実ログでは区切り線の直後に必ず ❯ 行が続き、そちらが安全網になるため）。
	workflowInputBoundaryRe = regexp.MustCompile(`^[\s\x{00A0}─━═]*[─━═]{3,}[\s\x{00A0}─━═]*$|❯`)
)

const (
	workflowSpinnerGlyphs = "⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏⣾⣽⣻⢿⡿⣟⣯⣷◐◓◑◒◜◝◞◟"
	workflowDoneGlyphs    = "✓✔"
	workflowFailedGlyphs  = "✗✘"
	workflowPendingGlyphs = "○◌◯"
)

func stripWorkflowANSI(line string) string {
	line = workflowOSCRe.ReplaceAllString(line, "")
	return workflowCSIRe.ReplaceAllString(line, "")
}

func stripWorkflowTree(line string) string {
	return workflowTreeRe.ReplaceAllString(stripWorkflowANSI(line), "")
}

func truncateWorkflowText(s string, maxRunes int) string {
	if maxRunes <= 0 {
		return ""
	}
	runes := []rune(s)
	if len(runes) <= maxRunes {
		return s
	}
	return string(runes[:maxRunes])
}

func workflowHeaderName(line string) (string, bool) {
	if workflowTipLineRe.MatchString(strings.TrimSpace(line)) {
		return "", false
	}
	hasGear := strings.Contains(line, "⚙")
	if !hasGear && !workflowHeaderRe.MatchString(line) {
		return "", false
	}
	name := strings.ReplaceAll(line, "⚙", "")
	name = workflowHeaderRe.ReplaceAllString(name, "")
	name = strings.TrimSpace(strings.TrimLeft(name, ":："))
	name = strings.Join(strings.Fields(name), " ")
	switch strings.ToLower(name) {
	case "running", "done", "complete", "completed", "in progress":
		name = ""
	}
	name = truncateWorkflowText(name, 60)
	return name, true
}

func workflowSummary(line string) (done, total, elapsed int, tokens string, ok bool) {
	m := workflowSummaryRe.FindStringSubmatch(line)
	if m == nil {
		return 0, 0, 0, "", false
	}
	done, _ = strconv.Atoi(m[1])
	total, _ = strconv.Atoi(m[2])
	if total <= 0 || done < 0 || done > total {
		return 0, 0, 0, "", false
	}
	tail := line[strings.Index(line, m[0])+len(m[0]):]
	elapsedFound := false
	for _, segment := range strings.FieldsFunc(tail, func(r rune) bool { return r == '·' || r == '•' }) {
		segment = strings.TrimSpace(segment)
		if strings.HasPrefix(segment, "↓") {
			tokens = segment
			continue
		}
		matches := workflowTimeRe.FindAllStringSubmatch(segment, -1)
		if len(matches) == 0 || elapsedFound {
			continue
		}
		for _, tm := range matches {
			v, _ := strconv.Atoi(tm[1])
			switch strings.ToLower(tm[2]) {
			case "h":
				elapsed += v * 3600
			case "m":
				elapsed += v * 60
			case "s":
				elapsed += v
			}
		}
		elapsedFound = true
	}
	return done, total, elapsed, tokens, true
}

func workflowAgent(line string) (proto.WfAgent, bool) {
	first, size := utf8.DecodeRuneInString(line)
	if first == utf8.RuneError || size == 0 {
		return proto.WfAgent{}, false
	}
	state := ""
	switch {
	case strings.ContainsRune(workflowSpinnerGlyphs, first) || first == '●':
		state = "running"
	case strings.ContainsRune(workflowDoneGlyphs, first):
		state = "done"
	case strings.ContainsRune(workflowFailedGlyphs, first):
		state = "failed"
	case strings.ContainsRune(workflowPendingGlyphs, first):
		state = "pending"
	default:
		return proto.WfAgent{}, false
	}
	rest := line[size:]
	if rest != "" && rest[0] != ' ' && rest[0] != '\t' {
		return proto.WfAgent{}, false
	}
	rest = strings.TrimSpace(rest)
	label, metrics := rest, ""
	if loc := workflowMetricsRe.FindStringIndex(rest); loc != nil {
		label = strings.TrimSpace(rest[:loc[0]])
		metrics = strings.TrimSpace(rest[loc[1]:])
	}
	label = strings.Join(strings.Fields(label), " ")
	if label == "" || len(label) > 200 {
		return proto.WfAgent{}, false
	}
	return proto.WfAgent{Label: label, State: state, Metrics: metrics}, true
}

func workflowWaiting(lines []string) int {
	for i := len(lines) - 1; i >= 0; i-- {
		m := workflowWaitingRe.FindStringSubmatch(stripWorkflowANSI(lines[i]))
		if m == nil {
			continue
		}
		n, _ := strconv.Atoi(m[1])
		if n > 0 {
			return n
		}
	}
	return 0
}

// parseWorkflowVT is a pure parser. The caller supplies plain VT mirror lines;
// ANSI stripping is repeated defensively so synthetic/raw fixtures are safe.
func parseWorkflowVT(lines []string) *proto.WorkflowProgress {
	if len(lines) == 0 {
		return nil
	}
	waiting := workflowWaiting(lines)
	start, name := -1, ""
	for i := len(lines) - 1; i >= 0; i-- {
		line := stripWorkflowTree(lines[i])
		if _, _, _, _, ok := workflowSummary(line); ok {
			start = i
			break
		}
		// The waiting sentinel contains the word "workflow", but it is not a
		// heading. Treating it as one would hide a valid tree immediately above.
		if workflowWaitingRe.MatchString(line) {
			continue
		}
		if n, ok := workflowHeaderName(line); ok {
			start, name = i, n
			break
		}
	}
	if start < 0 {
		if waiting > 0 {
			return &proto.WorkflowProgress{Detected: true, Source: "vt-summary", WaitingDynamic: waiting}
		}
		return nil
	}

	p := &proto.WorkflowProgress{Name: name, WaitingDynamic: waiting}
	var current *proto.WfPhase
	pendingTitle := ""
	explicitPercent := -1
	hasSummary := false
	for i := start; i < len(lines); i++ {
		// 見出し行より下で入力欄の区切りに達したら打ち切る（下は常時表示の UI）。
		if i > start && workflowInputBoundaryRe.MatchString(stripWorkflowANSI(lines[i])) {
			break
		}
		line := stripWorkflowTree(lines[i])
		if line == "" {
			continue
		}
		if done, total, elapsed, tokens, ok := workflowSummary(line); ok {
			p.Done, p.Total, p.Running = done, total, total-done
			p.ElapsedSec, p.TokensRaw = elapsed, tokens
			hasSummary = true
			continue
		}
		if i == start {
			continue
		}
		if agent, ok := workflowAgent(line); ok {
			if current == nil || pendingTitle != "" {
				p.Phases = append(p.Phases, proto.WfPhase{Title: pendingTitle})
				current = &p.Phases[len(p.Phases)-1]
				pendingTitle = ""
			}
			current.Agents = append(current.Agents, agent)
			continue
		}
		if m := workflowPercentRe.FindStringSubmatch(line); m != nil {
			v, _ := strconv.Atoi(m[1])
			if v <= 100 {
				explicitPercent = v
				continue
			}
		}
		pendingTitle = strings.Join(strings.Fields(line), " ")
		pendingTitle = truncateWorkflowText(pendingTitle, 80)
	}

	if hasSummary {
		p.Source = "vt-summary"
	} else {
		p.Source = "vt-tree"
		for _, phase := range p.Phases {
			for _, agent := range phase.Agents {
				p.Total++
				switch agent.State {
				case "running":
					p.Running++
				case "done":
					p.Done++
				case "failed":
					p.Failed++
				case "pending":
					p.Pending++
				}
			}
		}
	}
	if p.Total == 0 {
		if waiting > 0 {
			p.Detected = true
			p.Source = "vt-summary"
			return p
		}
		return nil
	}
	p.Detected = true
	if hasSummary {
		p.Percent = p.Done * 100 / p.Total
	} else if explicitPercent >= 0 {
		p.Percent = explicitPercent
	} else {
		p.Percent = (p.Done + p.Failed) * 100 / p.Total
	}
	return p
}

var crossSessionMessageHeaderRE = regexp.MustCompile(`(?i)^\s*[•●⏺]?\s*(message from|received message from)\s+(.+?)\s*$`)

type crossSessionMessageCandidate struct {
	Sender    string
	Line      string
	Signature string
}

func detectCrossSessionMessage(lines []string) *crossSessionMessageCandidate {
	var matches []string
	var sender string
	var line string
	for _, raw := range lines {
		trimmed := strings.TrimSpace(strings.TrimRight(raw, "\r"))
		if trimmed == "" {
			continue
		}
		parts := crossSessionMessageHeaderRE.FindStringSubmatch(trimmed)
		if len(parts) != 3 {
			continue
		}
		sender = strings.TrimSpace(parts[2])
		if sender == "" {
			continue
		}
		line = trimmed
		matches = append(matches, trimmed)
	}
	if len(matches) == 0 {
		return nil
	}
	return &crossSessionMessageCandidate{
		Sender:    sender,
		Line:      line,
		Signature: strings.Join(matches, "\n"),
	}
}


const workflowJournalFieldMax = 256
type workflowJournalEvent struct {
	Type    string `json:"type"`
	AgentID string `json:"agentId"`
}

// workflowJournalFileState lives only in a live session. Agent IDs are kept as
// in-memory sets solely to make append/restart processing idempotent.
type workflowJournalParserMode uint8

const (
	workflowJournalParserNeedRoot workflowJournalParserMode = iota
	workflowJournalParserNeedKey
	workflowJournalParserNeedColon
	workflowJournalParserNeedValue
	workflowJournalParserString
	workflowJournalParserPrimitive
	workflowJournalParserComposite
	workflowJournalParserNeedComma
	workflowJournalParserComplete
)

// workflowJournalRecordParser is a bounded JSON object scanner. It extracts
// only the two metadata strings used by the workflow counter and skips all
// other values byte by byte, including a multi-megabyte result string.
type workflowJournalRecordParser struct {
	mode            workflowJournalParserMode
	started         bool
	invalid         bool
	stringIsKey     bool
	stringEscape    bool
	compositeDepth  int
	compositeString bool
	compositeEscape bool
	currentKey      string
	captureField    string
	quoted          []byte
	typeValue       string
	agentID         string
}

func (parser *workflowJournalRecordParser) reset() {
	*parser = workflowJournalRecordParser{
		mode:    workflowJournalParserNeedRoot,
		started: true,
	}
}

func (parser *workflowJournalRecordParser) ensureStarted() {
	if !parser.started {
		parser.reset()
	}
}

func (parser *workflowJournalRecordParser) invalidate() {
	parser.invalid = true
	parser.quoted = nil
}

func (parser *workflowJournalRecordParser) beginString(isKey bool) {
	parser.mode = workflowJournalParserString
	parser.stringIsKey = isKey
	parser.stringEscape = false
	parser.captureField = ""
	if isKey {
		parser.quoted = parser.quoted[:0]
	} else if parser.currentKey == "type" || parser.currentKey == "agentId" {
		parser.captureField = parser.currentKey
		parser.quoted = parser.quoted[:0]
	} else {
		parser.quoted = nil
	}
}

func (parser *workflowJournalRecordParser) captureByte(b byte) {
	if !parser.stringIsKey && parser.captureField == "" {
		return
	}
	if len(parser.quoted) < workflowJournalFieldMax {
		parser.quoted = append(parser.quoted, b)
	}
}

func decodeWorkflowJournalQuoted(raw []byte) (string, bool) {
	if len(raw) > workflowJournalFieldMax {
		return "", false
	}
	encoded := make([]byte, 0, len(raw)+2)
	encoded = append(encoded, '"')
	encoded = append(encoded, raw...)
	encoded = append(encoded, '"')
	var value string
	if err := json.Unmarshal(encoded, &value); err != nil {
		return "", false
	}
	return value, true
}

func (parser *workflowJournalRecordParser) finishString() {
	value, ok := decodeWorkflowJournalQuoted(parser.quoted)
	if !ok {
		parser.invalidate()
		return
	}
	if parser.stringIsKey {
		parser.currentKey = value
		parser.mode = workflowJournalParserNeedColon
	} else {
		switch parser.captureField {
		case "type":
			parser.typeValue = value
		case "agentId":
			parser.agentID = value
		}
		parser.mode = workflowJournalParserNeedComma
	}
	parser.captureField = ""
	parser.quoted = nil
}

func (parser *workflowJournalRecordParser) feed(b byte) {
	parser.ensureStarted()
	if parser.invalid || parser.mode == workflowJournalParserComplete {
		return
	}
	if parser.mode == workflowJournalParserString {
		if parser.stringEscape {
			parser.captureByte(b)
			parser.stringEscape = false
			return
		}
		if b == '\\' {
			parser.captureByte(b)
			parser.stringEscape = true
			return
		}
		if b == '"' {
			parser.finishString()
			return
		}
		parser.captureByte(b)
		return
	}
	if parser.mode == workflowJournalParserComposite {
		if parser.compositeString {
			if parser.compositeEscape {
				parser.compositeEscape = false
			} else if b == '\\' {
				parser.compositeEscape = true
			} else if b == '"' {
				parser.compositeString = false
			}
			return
		}
		switch b {
		case '"':
			parser.compositeString = true
		case '{', '[':
			parser.compositeDepth++
		case '}', ']':
			parser.compositeDepth--
			if parser.compositeDepth <= 0 {
				parser.compositeDepth = 0
				parser.mode = workflowJournalParserNeedComma
			}
		}
		return
	}

	switch parser.mode {
	case workflowJournalParserNeedRoot:
		switch b {
		case ' ', '\t', '\r':
			return
		case '{':
			parser.mode = workflowJournalParserNeedKey
		default:
			parser.invalidate()
		}
	case workflowJournalParserNeedKey:
		switch b {
		case ' ', '\t', '\r', '\n':
			return
		case '"':
			parser.beginString(true)
		case '}':
			parser.mode = workflowJournalParserComplete
		default:
			parser.invalidate()
		}
	case workflowJournalParserNeedColon:
		if b == ' ' || b == '\t' || b == '\r' {
			return
		}
		if b == ':' {
			parser.mode = workflowJournalParserNeedValue
		} else {
			parser.invalidate()
		}
	case workflowJournalParserNeedValue:
		switch b {
		case ' ', '\t', '\r':
			return
		case '"':
			parser.beginString(false)
		case '{', '[':
			parser.mode = workflowJournalParserComposite
			parser.compositeDepth = 1
			parser.compositeString = false
			parser.compositeEscape = false
		case 't', 'f', 'n', '-', '0', '1', '2', '3', '4', '5', '6', '7', '8', '9':
			parser.mode = workflowJournalParserPrimitive
		default:
			parser.invalidate()
		}
	case workflowJournalParserPrimitive:
		switch b {
		case ',':
			parser.mode = workflowJournalParserNeedKey
		case '}':
			parser.mode = workflowJournalParserComplete
		case ' ', '\t', '\r':
			parser.mode = workflowJournalParserNeedComma
		}
	case workflowJournalParserNeedComma:
		switch b {
		case ' ', '\t', '\r':
			return
		case ',':
			parser.mode = workflowJournalParserNeedKey
		case '}':
			parser.mode = workflowJournalParserComplete
		default:
			parser.invalidate()
		}
	case workflowJournalParserComplete:
		if b != ' ' && b != '\t' && b != '\r' {
			parser.invalidate()
		}
	}
}

func (parser *workflowJournalRecordParser) event() (workflowJournalEvent, bool) {
	if parser == nil || parser.invalid || parser.mode != workflowJournalParserComplete || parser.typeValue == "" || parser.agentID == "" {
		return workflowJournalEvent{}, false
	}
	return workflowJournalEvent{Type: parser.typeValue, AgentID: parser.agentID}, true
}


func main(){ var cases []struct{ Name string `json:"name"`; Lines []string `json:"lines"`; Journal string `json:"journal"` };raw,err:=os.ReadFile(os.Args[1]);if err!=nil{panic(err)};if err=json.Unmarshal(raw,&cases);err!=nil{panic(err)};out:=[]map[string]any{};for _,row:=range cases{parser:=workflowJournalRecordParser{};for _,b:=range []byte(row.Journal){parser.feed(b)};event,ok:=parser.event();var journal any;if ok{journal=event};out=append(out,map[string]any{"name":row.Name,"workflow":parseWorkflowVT(row.Lines),"cross":detectCrossSessionMessage(row.Lines),"journal":journal})};if err=json.NewEncoder(os.Stdout).Encode(out);err!=nil{panic(err)}}
