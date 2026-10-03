// Run with an isolated Go cache. This helper imports only the standard library.
// It parses baseline source, feeds the actual flag declarations to Go's flag
// package, and records lexical observations. It never imports/runs the app.
package main

import (
	"bytes"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"errors"
	"flag"
	"go/ast"
	"go/format"
	"go/parser"
	"go/token"
	"os"
	"path/filepath"
	"sort"
	"strconv"
	"strings"
)

type Flag struct {
	Name    string `json:"name"`
	Kind    string `json:"kind"`
	Default any    `json:"default"`
	Usage   string `json:"usage"`
	Line    int    `json:"line"`
}
type FlagSet struct {
	Name     string `json:"name"`
	Source   string `json:"source"`
	Function string `json:"function"`
	Line     int    `json:"line"`
	Flags    []Flag `json:"flags"`
}
type Case struct {
	ID        string         `json:"id"`
	FlagSet   string         `json:"flag_set"`
	Args      []string       `json:"args"`
	Values    map[string]any `json:"values"`
	Remaining []string       `json:"remaining"`
	Error     string         `json:"error"`
	Help      bool           `json:"help"`
	Stderr    string         `json:"stderr"`
}
type Source struct {
	Path   string `json:"path"`
	SHA256 string `json:"sha256"`
}
type Read struct {
	Name       string `json:"name,omitempty"`
	Expression string `json:"expression"`
	Source     string `json:"source"`
	Line       int    `json:"line"`
	API        string `json:"api"`
	Resolution string `json:"resolution"`
}
type Dispatch struct {
	Names []string `json:"names"`
	Line  int      `json:"line"`
	Calls []string `json:"calls"`
}
type DispatchCase struct {
	ID        string   `json:"id"`
	Args      []string `json:"args"`
	Custom    []string `json:"custom_providers"`
	Route     string   `json:"route"`
	Provider  string   `json:"provider,omitempty"`
	Forwarded []string `json:"forwarded"`
	Error     string   `json:"error"`
}
type Output struct {
	Method           string         `json:"method"`
	Sources          []Source       `json:"sources"`
	FlagSets         []FlagSet      `json:"flag_sets"`
	FlagCases        []Case         `json:"flag_cases"`
	Dispatch         []Dispatch     `json:"dispatch"`
	DispatchCases    []DispatchCase `json:"dispatch_cases"`
	EnvironmentReads []Read         `json:"environment_reads"`
}

var fset = token.NewFileSet()
var trees = map[string]*ast.File{}
var constants = map[string]ast.Expr{}
var usedSources = map[string]bool{}

func expression(n ast.Node) string {
	var b bytes.Buffer
	if err := format.Node(&b, fset, n); err != nil {
		panic(err)
	}
	return b.String()
}
func literal(e ast.Expr, source string) (any, bool) {
	switch x := e.(type) {
	case *ast.BasicLit:
		if x.Kind == token.STRING {
			v, err := strconv.Unquote(x.Value)
			return v, err == nil
		}
		if x.Kind == token.INT {
			v, err := strconv.Atoi(x.Value)
			return v, err == nil
		}
	case *ast.Ident:
		if x.Name == "true" {
			return true, true
		}
		if x.Name == "false" {
			return false, true
		}
		if v, ok := constants[filepath.Dir(source)+"/"+x.Name]; ok {
			declSource := fset.Position(v.Pos()).Filename
			usedSources[declSource] = true
			return literal(v, declSource)
		}
	case *ast.BinaryExpr:
		if x.Op == token.ADD {
			a, ok := literal(x.X, source)
			b, ok2 := literal(x.Y, source)
			if ok && ok2 {
				sa, oka := a.(string)
				sb, okb := b.(string)
				if oka && okb {
					return sa + sb, true
				}
			}
		}
	case *ast.SelectorExpr:
		if pkg, ok := x.X.(*ast.Ident); ok {
			for _, imp := range trees[source].Imports {
				path, _ := strconv.Unquote(imp.Path.Value)
				name := filepath.Base(path)
				if imp.Name != nil {
					name = imp.Name.Name
				}
				if name == pkg.Name && strings.HasPrefix(path, "many-ai-cli/") {
					key := strings.TrimPrefix(path, "many-ai-cli/") + "/" + x.Sel.Name
					if v, ok := constants[key]; ok {
						declSource := fset.Position(v.Pos()).Filename
						usedSources[declSource] = true
						return literal(v, declSource)
					}
				}
			}
		}
	}
	return nil, false
}
func main() {
	root := flag.String("root", ".", "repository root")
	flag.Parse()
	var out Output
	out.Method = "Go standard-library flag parser with source AST declarations; source AST dispatch projection. No application runtime, home/config IO, provider processes, or network."
	var files []string
	for _, dir := range []string{"cmd", "internal"} {
		err := filepath.WalkDir(filepath.Join(*root, dir), func(path string, d os.DirEntry, err error) error {
			if err != nil {
				return err
			}
			if !d.IsDir() && strings.HasSuffix(path, ".go") && !strings.HasSuffix(path, "_test.go") {
				rel, _ := filepath.Rel(*root, path)
				files = append(files, filepath.ToSlash(rel))
			}
			return nil
		})
		if err != nil {
			panic(err)
		}
	}
	sort.Strings(files)
	for _, source := range files {
		data, err := os.ReadFile(filepath.Join(*root, source))
		if err != nil {
			panic(err)
		}
		tree, err := parser.ParseFile(fset, source, data, 0)
		if err != nil {
			panic(err)
		}
		trees[source] = tree
		for _, decl := range tree.Decls {
			g, ok := decl.(*ast.GenDecl)
			if !ok || g.Tok != token.CONST {
				continue
			}
			for _, sp := range g.Specs {
				v := sp.(*ast.ValueSpec)
				for i, n := range v.Names {
					if i < len(v.Values) {
						constants[filepath.Dir(source)+"/"+n.Name] = v.Values[i]
					}
				}
			}
		}
	}
	entry := map[string]bool{"cmd/many-ai-cli/main.go": true, "cmd/many-ai-cli/issue.go": true, "cmd/many-ai-cli/provider_command.go": true, "cmd/many-ai-cli-launcher/main.go": true, "internal/wrapper/wrapper.go": true, "internal/orchestrate/orchestrate.go": true, "internal/usagerelay/usagerelay.go": true}
	for _, source := range files {
		tree := trees[source]
		if entry[source] {
			data, _ := os.ReadFile(filepath.Join(*root, source))
			sum := sha256.Sum256(data)
			out.Sources = append(out.Sources, Source{source, hex.EncodeToString(sum[:])})
			for _, decl := range tree.Decls {
				fn, ok := decl.(*ast.FuncDecl)
				if !ok {
					continue
				}
				var current *FlagSet
				ast.Inspect(fn.Body, func(n ast.Node) bool {
					call, ok := n.(*ast.CallExpr)
					if !ok {
						return true
					}
					sel, ok := call.Fun.(*ast.SelectorExpr)
					if !ok {
						return true
					}
					x, ok := sel.X.(*ast.Ident)
					if !ok {
						return true
					}
					if x.Name == "flag" && sel.Sel.Name == "NewFlagSet" {
						name, ok := literal(call.Args[0], source)
						if !ok {
							panic("unresolved flagset")
						}
						out.FlagSets = append(out.FlagSets, FlagSet{Name: name.(string), Source: source, Function: fn.Name.Name, Line: fset.Position(call.Pos()).Line, Flags: []Flag{}})
						current = &out.FlagSets[len(out.FlagSets)-1]
					}
					if x.Name == "fs" && current != nil {
						kind := sel.Sel.Name
						if kind == "String" || kind == "Bool" || kind == "Int" {
							name, ok := literal(call.Args[0], source)
							def, ok2 := literal(call.Args[1], source)
							usage, ok3 := literal(call.Args[2], source)
							if !ok || !ok2 || !ok3 {
								panic("unresolved flag declaration: " + expression(call))
							}
							current.Flags = append(current.Flags, Flag{name.(string), strings.ToLower(kind), def, usage.(string), fset.Position(call.Pos()).Line})
						}
					}
					return true
				})
				if source == "cmd/many-ai-cli/main.go" && fn.Name.Name == "run" {
					ast.Inspect(fn.Body, func(n ast.Node) bool {
						sw, ok := n.(*ast.SwitchStmt)
						if !ok {
							return true
						}
						if expression(sw.Tag) != "cmd" {
							return true
						}
						for _, s := range sw.Body.List {
							c := s.(*ast.CaseClause)
							row := Dispatch{Names: []string{}, Line: fset.Position(c.Pos()).Line, Calls: []string{}}
							for _, e := range c.List {
								v, _ := literal(e, source)
								row.Names = append(row.Names, v.(string))
							}
							for _, s := range c.Body {
								ast.Inspect(s, func(n ast.Node) bool {
									r, ok := n.(*ast.ReturnStmt)
									if ok {
										for _, e := range r.Results {
											if call, ok := e.(*ast.CallExpr); ok {
												row.Calls = append(row.Calls, expression(call))
											}
										}
									}
									return true
								})
							}
							out.Dispatch = append(out.Dispatch, row)
						}
						return false
					})
				}
			}
		}
		ast.Inspect(tree, func(n ast.Node) bool {
			call, ok := n.(*ast.CallExpr)
			if !ok {
				return true
			}
			sel, ok := call.Fun.(*ast.SelectorExpr)
			if !ok {
				return true
			}
			x, ok := sel.X.(*ast.Ident)
			if !ok {
				return true
			}
			if (x.Name == "os" || x.Name == "deps") && (sel.Sel.Name == "Getenv" || sel.Sel.Name == "LookupEnv" || sel.Sel.Name == "getenv") && len(call.Args) > 0 {
				r := Read{Expression: expression(call.Args[0]), Source: source, Line: fset.Position(call.Pos()).Line, API: expression(call.Fun), Resolution: "dynamic expression; caller inventory required"}
				if v, ok := literal(call.Args[0], source); ok {
					r.Name, _ = v.(string)
					r.Resolution = "source constant or literal"
				}
				usedSources[source] = true
				out.EnvironmentReads = append(out.EnvironmentReads, r)
			}
			return true
		})
	}
	for _, src := range out.Sources {
		usedSources[src.Path] = true
	}
	out.Sources = nil
	for _, source := range files {
		if usedSources[source] {
			data, err := os.ReadFile(filepath.Join(*root, source))
			if err != nil {
				panic(err)
			}
			sum := sha256.Sum256(data)
			out.Sources = append(out.Sources, Source{source, hex.EncodeToString(sum[:])})
		}
	}
	for _, fs := range out.FlagSets {
		out.FlagCases = append(out.FlagCases, flagCases(fs)...)
	}
	out.DispatchCases = dispatchCases(out.Dispatch)
	enc := json.NewEncoder(os.Stdout)
	enc.SetIndent("", "  ")
	if err := enc.Encode(out); err != nil {
		panic(err)
	}
}
func observe(spec FlagSet, id string, args []string) Case {
	fs := flag.NewFlagSet(spec.Name, flag.ContinueOnError)
	var stderr bytes.Buffer
	fs.SetOutput(&stderr)
	for _, f := range spec.Flags {
		switch f.Kind {
		case "string":
			fs.String(f.Name, f.Default.(string), f.Usage)
		case "bool":
			fs.Bool(f.Name, f.Default.(bool), f.Usage)
		case "int":
			fs.Int(f.Name, f.Default.(int), f.Usage)
		default:
			panic("unknown kind")
		}
	}
	err := fs.Parse(args)
	c := Case{ID: spec.Name + "/" + id, FlagSet: spec.Name, Args: append([]string{}, args...), Values: map[string]any{}, Remaining: append([]string{}, fs.Args()...), Help: errors.Is(err, flag.ErrHelp), Stderr: stderr.String()}
	if err != nil {
		c.Error = err.Error()
	}
	fs.VisitAll(func(f *flag.Flag) { c.Values[f.Name] = f.Value.(flag.Getter).Get() })
	return c
}
func flagCases(s FlagSet) []Case {
	var out []Case
	add := func(id string, args ...string) { out = append(out, observe(s, id, args)) }
	add("default")
	add("short-help", "-h")
	add("long-help", "--help")
	add("help-inline", "--help=ignored")
	add("help-stops", "--help", "--unknown")
	add("unknown", "--unknown")
	add("bad-three-dashes", "---help")
	add("bad-empty-name", "--=value")
	add("bad-three-dashes-value", "---profile=value")
	add("end-options", "--", "--unknown")
	add("single-dash", "-", "--unknown")
	add("positional-stops", "日本語", "--unknown")
	for _, f := range s.Flags {
		switch f.Kind {
		case "bool":
			add(f.Name+"-bare", "--"+f.Name)
			add(f.Name+"-single-dash", "-"+f.Name)
			add(f.Name+"-separate-false", "--"+f.Name, "false", "--unknown")
			for _, v := range []string{"1", "t", "T", "true", "TRUE", "True", "0", "f", "F", "false", "FALSE", "False", "bad", "", "yes", "TrUe", "a\"b", "a\nb", "日本語"} {
				add(f.Name+"-inline-"+v, "--"+f.Name+"="+v)
			}
			add(f.Name+"-repeated", "--"+f.Name, "--"+f.Name+"=false")
		case "string":
			add(f.Name+"-space-value", "--"+f.Name, "two words 日本語")
			add(f.Name+"-inline", "-"+f.Name+"=value=tail")
			add(f.Name+"-empty", "--"+f.Name+"=")
			add(f.Name+"-missing", "--"+f.Name)
			add(f.Name+"-flag-value", "--"+f.Name, "--help")
			add(f.Name+"-terminator-value", "--"+f.Name, "--")
			add(f.Name+"-repeated", "--"+f.Name, "first", "--"+f.Name, "second")
		case "int":
			for _, v := range []string{"42", "0", "-1", "+42", "0x10", "010", "08", "1_000", "bad", "", "9223372036854775808"} {
				add(f.Name+"-inline-"+v, "--"+f.Name+"="+v)
			}
			add(f.Name+"-missing", "--"+f.Name)
			add(f.Name+"-separate-negative", "--"+f.Name, "-1")
		}
	}
	return out
}
func dispatchCases(rows []Dispatch) []DispatchCase {
	// This is a projection of source switch cases/call arguments, not an app run.
	var cases []DispatchCase
	add := func(id string, args, custom []string) {
		c := DispatchCase{ID: id, Args: append([]string{}, args...), Custom: append([]string{}, custom...), Forwarded: []string{}}
		if len(args) == 0 {
			c.Route = "default"
			cases = append(cases, c)
			return
		}
		name := args[0]
		matched := false
		for _, r := range rows {
			for _, n := range r.Names {
				if n != name {
					continue
				}
				matched = true
				first := r.Names[0]
				c.Route = first
				if first == "-h" {
					c.Route = "help"
				}
				if first == "claude" {
					c.Route = "wrap"
					c.Provider = name
					c.Forwarded = append(c.Forwarded, args[1:]...)
				} else if first == "wrap" {
					if len(args) < 2 {
						c.Error = "wrap <provider>"
					} else {
						c.Provider = args[1]
						c.Forwarded = append(c.Forwarded, args[2:]...)
					}
				} else if first == "orchestrate" && len(args) < 2 {
					c.Error = "orchestrate <spawn|send>"
				} else {
					for _, call := range r.Calls {
						if strings.Contains(call, "args[1:]") {
							c.Forwarded = append(c.Forwarded, args[1:]...)
							break
						}
					}
					if first == "serve" || first == "connect" || first == "profile-export" || first == "doctor" || first == "log-clean" || first == "uninstall" || first == "setup" {
						c.Forwarded = append([]string{}, args[1:]...)
					}
				}
			}
		}
		if !matched {
			for _, p := range custom {
				if p == name {
					matched = true
					c.Route = "wrap"
					c.Provider = name
					c.Forwarded = append(c.Forwarded, args[1:]...)
					break
				}
			}
			if !matched {
				c.Error = "unknown command: " + name
			}
		}
		cases = append(cases, c)
	}
	add("default", nil, nil)
	for _, r := range rows {
		for _, name := range r.Names {
			add("dispatch-"+name, []string{name}, nil)
			add("dispatch-"+name+"-tail", []string{name, "opaque", "--trial-root", "日本語"}, nil)
		}
	}
	add("configured-custom", []string{"synthetic-provider", "--opaque", "日本語"}, []string{"synthetic-provider"})
	add("unknown", []string{"not-a-command"}, nil)
	add("gemini-unconfigured", []string{"gemini"}, nil)
	add("gemini-configured", []string{"gemini", "--opaque"}, []string{"gemini"})
	add("reserved-verb-wins", []string{"serve", "--open"}, []string{"serve"})
	add("wrapper-unknown-provider-is-delegated", []string{"wrap", "not-configured", "--opaque"}, nil)
	return cases
}
