package hub

import "testing"

func TestParseGitStatusPorcelainZ(t *testing.T) {
	raw := " M web/src/app.js\x00?? internal/hub/git_status.go\x00D  old.txt\x00R  new.txt\x00old.txt\x00"
	files := parseGitStatusPorcelainZ(raw)
	if len(files) != 4 {
		t.Fatalf("len(files) = %d, want 4: %#v", len(files), files)
	}
	cases := []struct {
		status string
		path   string
	}{
		{"M", "web/src/app.js"},
		{"??", "internal/hub/git_status.go"},
		{"D", "old.txt"},
		{"R", "new.txt"},
	}
	for i, want := range cases {
		if files[i].Status != want.status || files[i].Path != want.path {
			t.Fatalf("files[%d] = {%q %q}, want {%q %q}", i, files[i].Status, files[i].Path, want.status, want.path)
		}
	}
}

func TestApplyWorkingTreeNumstat(t *testing.T) {
	files := []gitStatusFile{
		{Status: "M", Path: "web/src/app.js"},
		{Status: "R", Path: "new.txt"},
		{Status: "??", Path: "untracked.txt"},
	}
	applyWorkingTreeNumstat(files, "12\t3\tweb/src/app.js\n1\t0\told.txt => new.txt\n")
	if files[0].Added == nil || *files[0].Added != 12 || files[0].Removed == nil || *files[0].Removed != 3 {
		t.Fatalf("app.js stat = +%v -%v, want +12 -3", files[0].Added, files[0].Removed)
	}
	if files[1].Added == nil || *files[1].Added != 1 || files[1].Removed == nil || *files[1].Removed != 0 {
		t.Fatalf("rename stat = +%v -%v, want +1 -0", files[1].Added, files[1].Removed)
	}
	if files[2].Added != nil || files[2].Removed != nil {
		t.Fatalf("untracked stat = +%v -%v, want nil nil", files[2].Added, files[2].Removed)
	}
}

func TestParseAheadBehind(t *testing.T) {
	cases := []struct {
		raw           string
		ahead, behind int
		ok            bool
	}{
		{"0\t0\n", 0, 0, true},
		{"3\t2\n", 2, 3, true}, // behind=3, ahead=2
		{"5\t0", 0, 5, true},
		{"0\t4", 4, 0, true},
		{"", 0, 0, false},
		{"abc\t1", 0, 0, false},
		{"1", 0, 0, false},
		{"1\t2\t3", 0, 0, false},
	}
	for _, c := range cases {
		a, b, ok := parseAheadBehind(c.raw)
		if a != c.ahead || b != c.behind || ok != c.ok {
			t.Fatalf("parseAheadBehind(%q) = (%d, %d, %v), want (%d, %d, %v)",
				c.raw, a, b, ok, c.ahead, c.behind, c.ok)
		}
	}
}

func TestSuggestCommitMessage(t *testing.T) {
	subject, body := suggestCommitMessage([]gitStatusFile{
		{Status: "M", Path: "web/src/app.js"},
		{Status: "A", Path: "internal/hub/git_commit.go"},
	}, "2 files changed", "", "ja")
	if subject != "feat: webの変更を反映" {
		t.Fatalf("subject = %q", subject)
	}
	if body == "" {
		t.Fatalf("body is empty")
	}
}
