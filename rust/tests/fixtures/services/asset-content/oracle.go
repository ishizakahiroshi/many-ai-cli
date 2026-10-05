//go:build ignore

// Actual pinned Go stdlib observer; no Hub startup or filesystem outside this
// compile-time synthetic embed tree, and no network/service/account operation.
package main

import (
	"bytes"
	"embed"
	"encoding/json"
	"io/fs"
	"mime"
	"net/http"
	"net/http/httptest"
	"os"
	"path"
	"strings"
	"time"
)

//go:embed all:data
var embedded embed.FS

type input struct {
	Name           string      `json:"name"`
	Method         string      `json:"method"`
	Path           string      `json:"path"`
	Headers        [][2]string `json:"headers"`
	CompareContent bool        `json:"compare_content"`
}
type result struct {
	Status  int               `json:"status"`
	Headers map[string]string `json:"headers"`
	Body    []byte            `json:"body_base64"`
}
type observation struct {
	Name         string  `json:"name"`
	FileServer   result  `json:"file_server"`
	ServeContent *result `json:"serve_content,omitempty"`
	ContentType  string  `json:"content_type,omitempty"`
	Content      []byte  `json:"content_base64,omitempty"`
}

const normalizedBoundary = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"

func observe(c input, handler http.Handler) result {
	req := httptest.NewRequest(c.Method, "http://synthetic.invalid"+c.Path, nil)
	for _, h := range c.Headers {
		req.Header.Add(h[0], h[1])
	}
	w := httptest.NewRecorder()
	handler.ServeHTTP(w, req)
	response := w.Result()
	headers := map[string]string{}
	for name, values := range response.Header {
		if len(values) != 1 {
			panic("unexpected multiple response header values")
		}
		headers[name] = values[0]
	}
	body := w.Body.String()
	if value := headers["Content-Type"]; strings.HasPrefix(value, "multipart/byteranges;") {
		_, params, err := mime.ParseMediaType(value)
		if err != nil {
			panic(err)
		}
		boundary := params["boundary"]
		if len(boundary) != 60 || strings.Trim(boundary, "0123456789abcdef") != "" {
			panic("unexpected multipart boundary format")
		}
		// Both boundaries are length 60: Content-Length is left unchanged.
		headers["Content-Type"] = strings.ReplaceAll(value, boundary, normalizedBoundary)
		body = strings.ReplaceAll(body, boundary, normalizedBoundary)
	}
	return result{response.StatusCode, headers, []byte(body)}
}
func main() {
	raw, err := os.ReadFile(os.Args[1])
	if err != nil {
		panic(err)
	}
	var cases []input
	if err := json.Unmarshal(raw, &cases); err != nil {
		panic(err)
	}
	sub, err := fs.Sub(embedded, "data")
	if err != nil {
		panic(err)
	}
	server := http.FileServer(http.FS(sub))
	rows := []observation{}
	for _, c := range cases {
		row := observation{Name: c.Name, FileServer: observe(c, server)}
		if c.CompareContent {
			name := strings.TrimPrefix(c.Path, "/")
			info, err := fs.Stat(sub, name)
			if err != nil {
				panic(err)
			}
			if !info.ModTime().IsZero() || info.IsDir() {
				panic("expected embedded regular file with zero modification time")
			}
			data, err := fs.ReadFile(sub, name)
			if err != nil {
				panic(err)
			}
			observed := observe(c, http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
				http.ServeContent(w, r, info.Name(), time.Time{}, bytes.NewReader(data))
			}))
			row.ServeContent = &observed
			row.ContentType = mime.TypeByExtension(path.Ext(name))
			if row.ContentType == "" {
				panic("fixture must use a known explicit MIME type")
			}
			row.Content = data
		}
		rows = append(rows, row)
	}
	enc := json.NewEncoder(os.Stdout)
	enc.SetIndent("", "  ")
	if err := enc.Encode(rows); err != nil {
		panic(err)
	}
}
