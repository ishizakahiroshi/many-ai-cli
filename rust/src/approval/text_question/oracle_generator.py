"""Extract the fixed Go pure grammar without translating its algorithms.

Usage: python oracle_generator.py <repository-root> <oracle.go>
The parent runs pinned Go; the subagent does not build shared executables.
"""
from pathlib import Path
import sys

root, target = map(Path, sys.argv[1:3])
source = (root / "internal/hub/approval_text_question.go").read_text(encoding="utf-8")
pure = source[source.index("const ("):source.index("// ---- 記録を開く ----")]
begin = pure.index("func (q *textQuestion) candidateIdentity")
end = pure.index("func newTextQuestion", begin)
pure = pure[:begin] + pure[end:]
marker = (root / "internal/hub/approval_marker.go").read_text(encoding="utf-8")
begin = marker.index("func approvalMarkerSignature(")
signature = marker[begin:marker.index("\n}", begin) + 2]
header = '''package main
import (
    "crypto/sha256"
    "encoding/hex"
    "encoding/json"
    "os"
    "regexp"
    "sort"
    "strconv"
    "strings"
    "unicode"
    "unicode/utf8"
    "many-ai-cli/internal/proto"
    "many-ai-cli/internal/sessionlog"
)
const approvalMarkerOpen = "[" + "MANY-AI-CLI" + "]"
const approvalMarkerClose = "[/" + "MANY-AI-CLI" + "]"
'''
driver = '''
func main() {
    var cases []struct { Name string; Lines []string }
    if err := json.NewDecoder(os.Stdin).Decode(&cases); err != nil { panic(err) }
    out := make([]struct { Name string; Question *textQuestion; Lines []string }, 0, len(cases))
    for _, c := range cases {
        out = append(out, struct { Name string; Question *textQuestion; Lines []string }{
            c.Name, detectTextQuestion(c.Lines), ungluedApprovalLines(c.Lines),
        })
    }
    if err := json.NewEncoder(os.Stdout).Encode(out); err != nil { panic(err) }
}
'''
target.write_text(header + pure + signature + driver, encoding="utf-8", newline="\n")
