//go:build ignore

package main
import("strings";"unicode";"encoding/json";"os")
var usageProbeConfirmNeedles = []string{
	"itrustthisfolder",
	"isthisaprojectyoucreated",
	"allowexternalclaude.md",
	"allowexternalimports",
	"disableexternalimports",
}
var usageProbeDialogTargets = []string{
	"yes,itrustthisfolder",
	"yes,allowexternalimports",
}
func usageProbeDialogKey(lines []string) (key string, ok bool) {
	target := -1
	for i, line := range lines {
		compact := strings.ToLower(collapseWhitespace(line))
		for _, want := range usageProbeDialogTargets {
			if strings.Contains(compact, want) {
				target = i
				break
			}
		}
		if target >= 0 {
			break
		}
	}
	if target < 0 {
		return "", false
	}
	cursor := -1
	for i, line := range lines {
		if !usageProbeIsCursorLine(line) {
			continue
		}
		if cursor < 0 || absInt(i-target) < absInt(cursor-target) {
			cursor = i
		}
	}
	if cursor < 0 {
		return "", false
	}
	switch {
	case cursor == target:
		return "\r", true
	case cursor < target:
		return "\x1b[B", true
	default:
		return "\x1b[A", true
	}
}

func usageProbeIsCursorLine(line string) bool {
	trimmed := strings.TrimSpace(line)
	return strings.HasPrefix(trimmed, "❯") || strings.HasPrefix(trimmed, "›") || strings.HasPrefix(trimmed, ">")
}

func absInt(v int) int {
	if v < 0 {
		return -v
	}
	return v
}

func usageProbeConfirmDialog(screen string) bool {
	compact := strings.ToLower(collapseWhitespace(screen))
	if compact == "" {
		return false
	}
	for _, needle := range usageProbeConfirmNeedles {
		if strings.Contains(compact, needle) {
			return true
		}
	}
	return false
}

func collapseWhitespace(text string) string {
	var b strings.Builder
	b.Grow(len(text))
	for _, r := range text {
		if !unicode.IsSpace(r) {
			b.WriteRune(r)
		}
	}
	return b.String()
}
 
func main(){data,e:=os.ReadFile(os.Args[1]);if e!=nil{panic(e)};var cases [][]string;if e=json.Unmarshal(data,&cases);e!=nil{panic(e)};out:=[]any{};for _,lines:=range cases{key,ok:=usageProbeDialogKey(lines);out=append(out,map[string]any{"key":key,"ok":ok,"confirm":usageProbeConfirmDialog(strings.Join(lines,""))})};data,e=json.MarshalIndent(out,"","  ");if e!=nil{panic(e)};if e=os.WriteFile(os.Args[2],data,0600);e!=nil{panic(e)}}
