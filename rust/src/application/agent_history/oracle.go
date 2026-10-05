package main
import("bufio";"encoding/json";"os";"strings";"many-ai-cli/internal/sessionlog")
const grokHistoryLineMax=8*1024*1024
type grokChatMessage struct {
	Role string `json:"role"` // "user" | "assistant"
	Text string `json:"text"`
}
type grokChatLine struct {
	Type            string          `json:"type"`
	SyntheticReason string          `json:"synthetic_reason"`
	Content         json.RawMessage `json:"content"`
}
type grokContentPart struct {
	Type string `json:"type"`
	Text string `json:"text"`
}
func readGrokChatHistory(path string) ([]grokChatMessage, error) {
	f, err := os.Open(path)
	if err != nil {
		return nil, err
	}
	defer f.Close()

	var messages []grokChatMessage
	sc := bufio.NewScanner(f)
	sc.Buffer(make([]byte, 64*1024), grokHistoryLineMax)
	for sc.Scan() {
		line := sc.Bytes()
		if len(line) == 0 {
			continue
		}
		var ev grokChatLine
		if json.Unmarshal(line, &ev) != nil {
			continue
		}
		text := grokContentText(ev.Content)
		switch ev.Type {
		case "assistant":
			if t := strings.TrimSpace(text); t != "" {
				messages = append(messages, grokChatMessage{Role: "assistant", Text: sessionlog.MaskSecrets(t)})
			}
		case "user":
			if ev.SyntheticReason != "" {
				continue
			}
			query, ok := extractGrokUserQuery(text)
			if !ok {
				continue
			}
			messages = append(messages, grokChatMessage{Role: "user", Text: sessionlog.MaskSecrets(query)})
		}
	}
	if err := sc.Err(); err != nil {
		return nil, err
	}
	return messages, nil
}
func grokContentText(raw json.RawMessage) string {
	if len(raw) == 0 {
		return ""
	}
	var s string
	if json.Unmarshal(raw, &s) == nil {
		return s
	}
	var parts []grokContentPart
	if json.Unmarshal(raw, &parts) != nil {
		return ""
	}
	var b strings.Builder
	for _, p := range parts {
		if p.Type == "text" && p.Text != "" {
			if b.Len() > 0 {
				b.WriteString("\n")
			}
			b.WriteString(p.Text)
		}
	}
	return b.String()
}
func extractGrokUserQuery(text string) (string, bool) {
	const openTag = "<user_query>"
	const closeTag = "</user_query>"
	start := strings.Index(text, openTag)
	if start < 0 {
		return "", false
	}
	rest := text[start+len(openTag):]
	end := strings.Index(rest, closeTag)
	if end < 0 {
		end = len(rest)
	}
	q := strings.TrimSpace(rest[:end])
	if q == "" {
		return "", false
	}
	return q, true
}
type Case struct{Name,Body string}
func main(){var cases []Case;if err:=json.NewDecoder(os.Stdin).Decode(&cases);err!=nil{panic(err)};out:=[]map[string]any{}
for _,c:=range cases{f,err:=os.CreateTemp("","many-ai-grok-history-oracle-");if err!=nil{panic(err)};path:=f.Name();if _,err:=f.WriteString(c.Body);err!=nil{panic(err)};f.Close();messages,err:=readGrokChatHistory(path);os.Remove(path);out=append(out,map[string]any{"name":c.Name,"messages":messages,"error":err!=nil})}
if err:=json.NewEncoder(os.Stdout).Encode(out);err!=nil{panic(err)}}
