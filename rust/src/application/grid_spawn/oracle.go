package main
import("encoding/json";"fmt";"os")
func calcGridLayout(count int) string {
	switch {
	case count <= 1:
		return "1x1"
	case count <= 2:
		return "1x2"
	case count <= 4:
		return "2x2"
	case count <= 6:
		return "2x3"
	case count <= 9:
		return "3x3"
	case count <= 12:
		return "4x3"
	default:
		return "6x3"
	}
}
type Input struct {Preset string;Count int;LabelPrefix string;Provider string}
func main(){
out:=[]map[string]any{}
for _,preset:=range []string{"shell","ai+shell"}{
for count:=1;count<=18;count++{
for _,prefix:=range []string{"","日本語"}{
body:=Input{preset,count,prefix,"custom-ai"}
aiProvider:=body.Provider
	type sessionSpec struct {
		provider string
		label    string
	}
	var specs []sessionSpec
	labelPrefix := body.LabelPrefix
	if labelPrefix == "" {
		labelPrefix = "grid"
	}

	switch body.Preset {
	case "shell":
		for i := 0; i < body.Count; i++ {
			specs = append(specs, sessionSpec{
				provider: "shell",
				label:    fmt.Sprintf("%s-%d", labelPrefix, i+1),
			})
		}
	case "ai+shell":
		// AI 1 枚 + Shell (count-1) 枚
		aiCount := 1
		shellCount := body.Count - aiCount
		if shellCount < 0 {
			shellCount = 0
		}
		specs = append(specs, sessionSpec{
			provider: aiProvider,
			label:    fmt.Sprintf("%s-%s-1", labelPrefix, aiProvider),
		})
		for i := 0; i < shellCount; i++ {
			specs = append(specs, sessionSpec{
				provider: "shell",
				label:    fmt.Sprintf("%s-shell-%d", labelPrefix, i+1),
			})
		}
	}

rows:=[][2]string{}
for _,spec:=range specs {rows=append(rows,[2]string{spec.provider,spec.label})}
out=append(out,map[string]any{"preset":preset,"count":count,"label_prefix":prefix,"provider":body.Provider,"layout":calcGridLayout(count),"specs":rows})
}}}
if err:=json.NewEncoder(os.Stdout).Encode(out);err!=nil{panic(err)}
}
