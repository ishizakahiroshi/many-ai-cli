#!/usr/bin/env python3
"""Regenerate only synthetic VT fixtures from fixed Go source (no Hub/provider).
Run from the repository root with an explicitly selected Go compiler, e.g.:
  GOTOOLCHAIN=local python3 rust/tests/fixtures/core/terminal/generate_oracle.py \
    --go /path/to/authorized/go --work-root /path/to/isolated/temp
The caller supplies isolated Go cache/module environment variables.
"""
import argparse
import json
from pathlib import Path
import random
import subprocess
import tempfile

ORACLE = "21d0bc7935a2c4696fb89ccff2e324157a528c2d"
MAIN = r'''package main
import ("encoding/json";"os")
type Case struct { Name string `json:"name"`; Cols int `json:"cols"`; Rows int `json:"rows"`; Bytes string `json:"bytes"`; Lines []string `json:"lines"`; Row int `json:"row"`; Col int `json:"col"`; Alt bool `json:"alt"` }
func main() { raw,err:=os.ReadFile(os.Args[1]); if err!=nil { panic(err) }; var cases []Case; if err=json.Unmarshal(raw,&cases);err!=nil{panic(err)}; for i:=range cases { c:=&cases[i];v:=newVTBuffer(c.Cols,c.Rows); v.Write([]byte(c.Bytes));c.Lines=v.TailLinesWithScrollback(0);c.Row=v.row;c.Col=v.col;c.Alt=v.altScreen }; json.NewEncoder(os.Stdout).Encode(cases) }
'''

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--go", required=True)
    parser.add_argument("--work-root", required=True, type=Path)
    args = parser.parse_args()
    if not Path(args.go).is_absolute() or not args.work_root.is_absolute():
        parser.error("Go executable and isolated work root must be absolute paths")
    repo = Path(__file__).resolve().parents[5]
    source = subprocess.check_output(["git", "show", f"{ORACLE}:internal/hub/vt_buffer.go"], cwd=repo, text=True)
    rng = random.Random(214)
    ops = ['a','日本','ÁB','😀','─','\r','\n','\b','\t','\x1b[2J','\x1b[K','\x1b[1K','\x1b[2K','\x1b[3X','\x1b[999X','\x1b[2;3H','\x1b[1A','\x1b[2B','\x1b[3C','\x1b[2D','\x1b[4G','\x1b7','\x1b8','\x1b[s','\x1b[u','\x1b[?1049h','\x1b[?1049l','\x1b(B','\x1b]hidden\x07','\x1bPhidden\x1b\\']
    cases = [{'name':f'oracle_{i:03}', 'cols':rng.randint(2,20), 'rows':rng.randint(1,5), 'bytes':''.join(rng.choices(ops,k=rng.randint(3,35)))} for i in range(200)]
    args.work_root.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="many-ai-vt-oracle-", dir=args.work_root) as temporary:
        root=Path(temporary)
        (root/"vt.go").write_text(source.replace("package hub", "package main", 1))
        (root/"main.go").write_text(MAIN)
        (root/"cases.json").write_text(json.dumps(cases,ensure_ascii=False))
        output = subprocess.check_output([args.go,"run",str(root/"main.go"),str(root/"vt.go"),str(root/"cases.json")],cwd=root)
    Path(__file__).with_name("vt-oracle-21d0bc7.json").write_bytes(output)

if __name__ == "__main__":
    main()
