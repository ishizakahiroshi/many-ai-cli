package main
import("fmt";"unicode")
func main(){fmt.Println("// Pinned Go Unicode 15 SimpleFold equivalence minima.");fmt.Println("pub const FOLD: &[(u32,u32)] = &[");for r:=rune(0);r<=unicode.MaxRune;r++{m:=r;for v:=unicode.SimpleFold(r);v!=r;v=unicode.SimpleFold(v){if v<m{m=v}};if m!=r{fmt.Printf("(%d,%d),\n",r,m)}};fmt.Println("];")}
