#!/usr/bin/env python3
"""Extract the fixed Go autoapproval package into an isolated synthetic oracle.

The only altered function is Path: it uses a harness-selected synthetic path.
Original summary and private atomic-write helpers are imported unchanged from
this checkout and checked against the oracle SHA. No home lookup, server, shell
command or provider request is performed by the program.
"""
from pathlib import Path
import subprocess

SHA = "21d0bc7935a2c4696fb89ccff2e324157a528c2d"
ROOT = Path(__file__).resolve().parents[5]
OUT = Path(__file__).resolve().parent

def source(path):
    return subprocess.check_output(["git", "show", f"{SHA}:{path}"], cwd=ROOT, text=True)

for path in ["internal/approval/summary.go", "internal/securefile/atomic.go"]:
    if (ROOT / path).read_text() != source(path):
        raise SystemExit(f"oracle dependency differs from pinned source: {path}")
policy = source("internal/autoapproval/policy.go")
policy = policy.replace("package autoapproval", "package main", 1)
policy = policy.replace('"many-ai-cli/internal/config"', '"encoding/json"')
start = policy.index("func Path()")
end = policy.index("\n}\n", start) + 3
policy = policy[:start] + 'var fixturePath string\nfunc Path() (string, error) { return fixturePath, nil }\n' + policy[end:]
harness = r'''
type input struct {
    Command string `json:"command"`
    CWD string `json:"cwd"`
    Risk proto.ApprovalRiskTier `json:"risk"`
}
type loadObservation struct {
    Name string `json:"name"`
    Raw string `json:"raw"`
    Missing bool `json:"missing"`
    Rules int `json:"rules"`
    Warnings []string `json:"warnings"`
    Error bool `json:"error"`
    Inputs []input `json:"inputs"`
    Decisions []Decision `json:"decisions"`
}
type addObservation struct {
    Name string `json:"name"`
    Initial string `json:"initial"`
    Command string `json:"command"`
    CWD string `json:"cwd"`
    Rule Rule `json:"rule"`
    Error string `json:"error"`
    Changed bool `json:"changed"`
    File File `json:"file"`
}
type regexObservation struct {
    Pattern string `json:"pattern"`
    Values []string `json:"values"`
    Valid bool `json:"valid"`
    Matches []bool `json:"matches"`
    Unsupported string `json:"unsupported,omitempty"`
}
func main() {
    root, err := os.MkdirTemp("", "many-ai-autoapproval-oracle-")
    if err != nil { panic(err) }
    defer os.RemoveAll(root)
    common := []input{
        {"git status", "/synthetic/project", "low"},
        {" git status \t", "/synthetic/project", "low"},
        {"git status", "/synthetic/project/child", "low"},
        {"git status", "/synthetic/project", "mid"},
        {"git status", "/synthetic/project", "high"},
        {"git status", "/synthetic/project", ""},
        {"git diff", "/synthetic/project", "low"},
        {"", "/synthetic/project", "high"},
        {"git push", "/synthetic/project", "mid"},
    }
    loads := []loadObservation{
        {Name:"missing", Missing:true},
        {Name:"empty", Raw:""},
        {Name:"null", Raw:"null\n"},
        {Name:"ordered_cwd", Raw:"version: 1\nrules:\n - id: scoped\n   command: '^git status$'\n   working_dir: '^/synthetic/project$'\n - id: fallback\n   command: '^git status$'\n   risk: [low, low]\n"},
        {Name:"historical_unknown_version", Raw:"version: 9\nfuture: {.nan: [true, 2026-01-02]}\nrules:\n - id: 001\n   command: '^git status$'\n   future: {42: .inf}\n"},
        {Name:"null_and_lexical", Raw:"version: 1.9\nrules:\n - null\n - id: true\n   command: '^git status$'\n   risk: [null, low]\n   working_dir: null\n"},
        {Name:"alias_merge", Raw:"base: &base {id: merged, command: '^git status$', risk: [low]}\nversion: 1\nrules:\n - <<: *base\n   working_dir: '^/synthetic/project$'\n"},
        {Name:"ordered_invalid", Raw:"version: 0\nrules:\n - command: '^git status$'\n - id: reserved\n - id: reserved\n   command: '^git status$'\n - id: syntax\n   command: '('\n - id: broad\n   command: '.*'\n - id: badcwd\n   command: '^git status$'\n   working_dir: '('\n - id: risk\n   command: '^git status$'\n   risk: [low, mid]\n - id: okay\n   command: '^git diff$'\n"},
        {Name:"malformed", Raw:"version: [broken"},
        {Name:"wrong_type", Raw:"version: 1\nrules: [false, {id: okay, command: '^git status$'}]\n"},
        {Name:"duplicate_field", Raw:"version: 1\nversion: 1\nrules: []\n"},
        {Name:"first_document", Raw:"version: 1\nrules: [{id: first, command: '^git status$'}]\n---\n[malformed"},
        {Name:"unicode_literal", Raw:"version: 1\nrules: [{id: cjk, command: '^cat 日本語.txt$', working_dir: '^/合成/作業$'}]\n", Inputs:[]input{{"cat 日本語.txt", "/合成/作業", "low"}, {"cat 日本語Xtxt", "/合成/作業", "low"}}},
        {Name:"unsupported_unicode_property", Raw:"version: 1\nrules: [{id: han, command: '^cat \\p{Han}+$'}]\n", Inputs:[]input{{"cat 日本語", "/synthetic", "low"}}},
    }
    for i := range loads {
        c := &loads[i]
        fixturePath = filepath.Join(root, fmt.Sprintf("load-%d.yaml", i))
        if !c.Missing { if err := os.WriteFile(fixturePath, []byte(c.Raw), 0600); err != nil { panic(err) } }
        p, err := Load()
        c.Error, c.Rules, c.Warnings = err != nil, len(p.Rules), p.Warnings
        if c.Inputs == nil { c.Inputs = common }
        for _, in := range c.Inputs { c.Decisions = append(c.Decisions, p.Evaluate(in.Command, in.CWD, in.Risk)) }
    }
    adds := []addObservation{
        {Name:"fresh", Command:" git status \t", CWD:"/synthetic/project"},
        {Name:"cwd_literal", Command:"cat 日本語.[txt]", CWD:`C:\synthetic\project.(demo)`},
        {Name:"empty_cwd", Command:"go test ./..."},
        {Name:"unsafe", Command:"git push origin main"},
        {Name:"empty", Command:" \t"},
        {Name:"multiline", Command:"git status\ngit diff"},
        {Name:"carriage_return", Command:"git status\rgit diff"},
        {Name:"invalid_file", Initial:"rules: [", Command:"git status"},
        {Name:"unknown_version", Initial:"version: 7\nextension: {ignored: .nan}\nrules: [{id: historic, command: '^git diff$', risk: [mid]}]\n", Command:"git status"},
        {Name:"zero_version", Initial:"version: 0\nrules: []\n", Command:"git status"},
        {Name:"reuse_invalid_existing", Initial:"version: 0\nextension: keep-me\nrules: [{id: old, command: '^git status$', risk: [high]}]\n", Command:"git status"},
    }
    for i := range adds {
        c := &adds[i]
        fixturePath = filepath.Join(root, fmt.Sprintf("add-%d.yaml", i))
        if c.Initial != "" { if err := os.WriteFile(fixturePath, []byte(c.Initial), 0600); err != nil { panic(err) } }
        var err error
        c.Rule, err = AddRule(c.Command, c.CWD)
        if err != nil { c.Error = err.Error() }
        if data, err := os.ReadFile(fixturePath); err == nil {
            c.Changed = string(data) != c.Initial
            _ = yaml.Unmarshal(data, &c.File)
        }
    }
    regexes := []regexObservation{
        {Pattern:`^cat \w+$`, Values:[]string{"cat abc_12", "cat 日本語", "cat café", "cat K"}},
        {Pattern:`^cat \s+file$`, Values:[]string{"cat  file", "cat \tfile", "cat \vfile", "cat \u00a0file"}},
        {Pattern:`^cat \S+$`, Values:[]string{"cat abc", "cat a\u00a0b", "cat a\vb", "cat a b"}},
        {Pattern:`\bgit status\b`, Values:[]string{"git status", "é git status é", "é git statusé", "xgit status"}},
        {Pattern:`^cat [a&&b]+$`, Values:[]string{"cat a", "cat b", "cat &", "cat ab&", "cat c"}},
        {Pattern:`^cat [a[b]+$`, Values:[]string{"cat [ab", "cat c"}},
        {Pattern:`^cat [^\W]+$`, Values:[]string{"cat abc_2", "cat 日本語", "cat &"}},
        {Pattern:`^cat [[:alpha:]]+$`, Values:[]string{"cat abc", "cat 日本語", "cat café"}},
        {Pattern:`^\Qcat (a+b).[txt]\E$`, Values:[]string{"cat (a+b).[txt]", "cat abXtxt"}},
        {Pattern:`^cat \141\x62\x{63}$`, Values:[]string{"cat abc", "cat 141bc"}},
        {Pattern:`^(?P<1name>cat) (?P<1name>file)$`, Values:[]string{"cat file", "cat"}},
        {Pattern:`^(?<same>cat) (?<same>file)$`, Values:[]string{"cat file"}},
        {Pattern:`(?i)^cat [a-z]+$`, Values:[]string{"CAT file", "cat Kſ", "cat 日"}},
        {Pattern:`^cat a{01}$`, Values:[]string{"cat a{01}", "cat a"}},
        {Pattern:`^cat a{,3}$`, Values:[]string{"cat a{,3}", "cat aaa"}},
        {Pattern:`^cat a{2,3}$`, Values:[]string{"cat a", "cat aa", "cat aaa", "cat aaaa"}},
        {Pattern:`(?m)^cat file$`, Values:[]string{"before\ncat file\nafter", "cat file\r\n", "cat file\n"}},
        {Pattern:`^cat [a--b]$`}, {Pattern:`^cat a{1001}$`},
        {Pattern:`^cat (a{20}){60}$`}, {Pattern:`^cat a++$`},
        {Pattern:`(?x)^cat file$`}, {Pattern:`(?u)^cat file$`},
        {Pattern:`^cat \u0061$`}, {Pattern:`^cat [\b]$`},
        {Pattern:`^cat \p{Han}+$`, Values:[]string{"cat 日本語"}, Unsupported:"unicode_property"},
        {Pattern:`(?i)^cat λ$`, Values:[]string{"cat Λ"}, Unsupported:"non_ascii_casefold"},
        {Pattern:`^cat \x{d800}$`, Values:[]string{"cat a"}, Unsupported:"surrogate"},
    }

    for _, pattern := range []string{
        `(?ii)^cat file$`, `(?i-i)^cat file$`, `(?im-ms)^cat file$`, `(?)^cat file$`,
        `^cat(?)*$`, `^cat [\D]+$`, `^cat [a\-z]+$`, `^cat []a-]+$`,
        `^cat [^]a]+$`, `^cat [[:^alpha:]]+$`, `^cat [[:bogus:]]+$`,
        `^cat \12$`, `^cat \1$`, `^cat \8$`, `^cat \0$`,
        `^cat a{2,}$`, `^cat (a{20}){0,}$`, `^cat (a{20}){0}$`,
        `^cat a{3,2}$`, `^cat a{0002}$`, `^cat a{2,03}$`,
        `^cat \p{Definitely_Not_A_Go_Class}$`, `^cat \p{Alphabetic}$`,
        `^cat \p{Script=Latin}$`, `^cat \x{110000}$`,
    } {
        regexes = append(regexes, regexObservation{Pattern:pattern, Values:[]string{
            "cat", "cat file", "CAT FILE", "cat a", "cat aaa", "cat 2", "cat !", "cat -",
            "cat ]", "cat ", "cat a{0002}", "cat a{2,03}", "cat \n", "cat \x00",
        }})
    }
    for i := range regexes {
        c := &regexes[i]
        re, err := regexp.Compile(c.Pattern)
        c.Valid = err == nil
        if err == nil { for _, value := range c.Values { c.Matches = append(c.Matches, re.MatchString(value)) } }
    }
    enc := json.NewEncoder(os.Stdout)
    enc.SetIndent("", "  ")
    if err := enc.Encode(map[string]any{"go_sha":"21d0bc7935a2c4696fb89ccff2e324157a528c2d", "loads":loads, "adds":adds, "regexes":regexes}); err != nil { panic(err) }
}
'''
(OUT / "go_oracle.go").write_text("//go:build ignore\n\n// Generated from fixed Go oracle " + SHA + ".\n" + policy + harness)
