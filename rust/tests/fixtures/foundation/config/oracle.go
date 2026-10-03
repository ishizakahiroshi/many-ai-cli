// Synthetic config encoding oracle; never reads HOME or user configuration.
package main

import (
	"encoding/json"
	"fmt"
	"gopkg.in/yaml.v3"
	"io"
	"many-ai-cli/internal/config"
	"os"
	"reflect"
)

func fill(v reflect.Value) {
	switch v.Kind() {
	case reflect.String:
		v.SetString("synthetic-日本語")
	case reflect.Bool:
		v.SetBool(true)
	case reflect.Int, reflect.Int64:
		v.SetInt(17)
	case reflect.Pointer:
		v.Set(reflect.New(v.Type().Elem()))
		fill(v.Elem())
	case reflect.Slice:
		v.Set(reflect.MakeSlice(v.Type(), 1, 1))
		fill(v.Index(0))
	case reflect.Map:
		v.Set(reflect.MakeMap(v.Type()))
		key := reflect.New(v.Type().Key()).Elem()
		key.SetString("synthetic")
		x := reflect.New(v.Type().Elem()).Elem()
		fill(x)
		v.SetMapIndex(key, x)
	case reflect.Struct:
		for i := 0; i < v.NumField(); i++ {
			fill(v.Field(i))
		}
	default:
		panic(v.Kind())
	}
}
func main() {
	var c config.Config
	if len(os.Args) > 1 {
		if len(os.Args) != 2 || os.Args[1] != "--stdin" {
			fmt.Fprintln(os.Stderr, "usage: oracle [--stdin]")
			os.Exit(2)
		}
		data, e := io.ReadAll(io.LimitReader(os.Stdin, 8*1024*1024+1))
		if len(data) > 8*1024*1024 {
			fmt.Fprintln(os.Stderr, "synthetic YAML exceeds limit")
			os.Exit(1)
		}
		if e != nil {
			panic(e)
		}
		if e = yaml.Unmarshal(data, &c); e != nil {
			fmt.Fprintln(os.Stderr, "synthetic YAML parse failed")
			os.Exit(1)
		}
	} else {
		fill(reflect.ValueOf(&c).Elem())
	}
	c.Token = ""
	c.AuthCookieSecret = ""
	c.RemotePINHash = ""
	bytes, e := json.Marshal(c)
	if e != nil {
		panic(e)
	}
	var value map[string]any
	if e = json.Unmarshal(bytes, &value); e != nil {
		panic(e)
	}
	delete(value, "Token")
	enc := json.NewEncoder(os.Stdout)
	enc.SetIndent("", "  ")
	if e = enc.Encode(value); e != nil {
		panic(e)
	}
}
