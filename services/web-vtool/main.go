// Command web-vtool is the presentation half of the smah **web** virtual
// tool (#19). The LLM seam lives in the Rust driver (crates/smah-core
// drivers/web.rs), which fabricates the page content as a strings-only
// content.json; this service only renders it, exposes a refresh button that
// touches regen.request (which the driver's watch loop polls), and pushes
// reload events to the browser over Server-Sent Events. It holds no
// providers, makes no model calls, and performs no network egress.
//
// Threat-model controls implemented here:
//   - T-019 (XSS / HTML injection): every fabricated string renders through
//     html/template auto-escaping server-side and textContent client-side —
//     never raw HTML — and a restrictive Content-Security-Policy backs both.
//     The push and regen channels are origin-checked.
//   - T-020 (loopback-only bind): the service REFUSES to start on any
//     non-loopback address and discloses the bind loudly on stderr.
package main

import (
	"embed"
	"encoding/json"
	"errors"
	"flag"
	"fmt"
	"html/template"
	"log"
	"net"
	"net/http"
	"net/url"
	"os"
	"path/filepath"
	"sort"
	"strings"
	"time"
)

//go:embed assets
var assets embed.FS

var indexTpl = template.Must(template.New("index").Parse(indexHTML))

const indexHTML = `<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>{{.Tool}} — fabricated virtual tool</title>
<link rel="stylesheet" href="/assets/semtext-mock-web.css">
<script src="/assets/htmx.min.js" defer></script>
<script src="/assets/semtext-mock-web.js" defer></script>
</head>
<body>
<semtext-page>
<header>
  <h1 id="page-title">{{.Title}}</h1>
  {{with .Notice}}<p class="notice">{{.}}</p>{{end}}
  <p class="banner">Fabricated content from the smah virtual tool
  &ldquo;{{.Tool}}&rdquo;. Loopback only — nothing on this page is real
  data.</p>
  <button id="regen" type="button" hx-post="/regen" hx-target="#regen-status"
          hx-swap="innerHTML">Ask the driver to regenerate</button>
  <span id="regen-status" role="status"></span>
</header>
<main id="sections">
  {{range .Sections}}<section><h2>{{.Key}}</h2><p>{{.Value}}</p></section>
  {{end}}
</main>
</semtext-page>
</body>
</html>
`

type section struct {
	Key   string
	Value string
}

type pageData struct {
	Tool     string
	Title    string
	Notice   string
	Sections []section
}

type server struct {
	contentPath string
	regenPath   string
	tool        string
}

// routes wires the handler set; shared by main and the tests.
func (s *server) routes() http.Handler {
	mux := http.NewServeMux()
	mux.HandleFunc("GET /{$}", s.handleIndex)
	mux.HandleFunc("GET /content.json", s.handleContent)
	mux.HandleFunc("POST /regen", s.handleRegen)
	mux.HandleFunc("GET /push", s.handlePush)
	mux.Handle("GET /assets/", http.FileServerFS(assets))
	return mux
}

// loadContent parses content.json into ordered sections. A strings-only JSON
// object is the format contract shared with the Rust driver's validation —
// anything else is an error, never a best-effort render.
func loadContent(path string) ([]section, string, error) {
	raw, err := os.ReadFile(path)
	if err != nil {
		return nil, "", err
	}
	var obj map[string]string
	if err := json.Unmarshal(raw, &obj); err != nil {
		return nil, "", fmt.Errorf("content is not a strings-only JSON object: %w", err)
	}
	title := obj["title"]
	keys := make([]string, 0, len(obj))
	for k := range obj {
		if k != "title" {
			keys = append(keys, k)
		}
	}
	sort.Strings(keys)
	secs := make([]section, 0, len(keys))
	for _, k := range keys {
		secs = append(secs, section{Key: k, Value: obj[k]})
	}
	return secs, title, nil
}

func (s *server) handleIndex(w http.ResponseWriter, _ *http.Request) {
	w.Header().Set("X-Content-Type-Options", "nosniff")
	w.Header().Set("Content-Security-Policy",
		"default-src 'self'; img-src 'self' data:; style-src 'self' 'unsafe-inline'; connect-src 'self'; script-src 'self'")
	secs, title, err := loadContent(s.contentPath)
	data := pageData{Tool: s.tool, Sections: secs, Title: title}
	if err != nil {
		data.Title = "content not ready"
		data.Notice = fmt.Sprintf(
			"content.json is not readable yet (%v). Run the smah web driver, then refresh.", err)
	}
	if err := indexTpl.Execute(w, data); err != nil {
		log.Printf("web-vtool: index render: %v", err)
	}
}

func (s *server) handleContent(w http.ResponseWriter, _ *http.Request) {
	raw, err := os.ReadFile(s.contentPath)
	if err != nil {
		http.Error(w, "content not available", http.StatusServiceUnavailable)
		return
	}
	w.Header().Set("Content-Type", "application/json")
	w.Header().Set("X-Content-Type-Options", "nosniff")
	_, _ = w.Write(raw)
}

// handleRegen touches regen.request — the filesystem signal the Rust
// driver's watch loop polls. Loopback + same-origin only: this endpoint can
// spend the driver's model-turn budget, so a random web page must not be
// able to fire it (T-019).
func (s *server) handleRegen(w http.ResponseWriter, r *http.Request) {
	if !guardLoopback(w, r) {
		return
	}
	now := time.Now()
	if err := os.Chtimes(s.regenPath, now, now); err != nil {
		if err := os.WriteFile(s.regenPath, []byte("regen\n"), 0o600); err != nil {
			http.Error(w, "cannot request regeneration", http.StatusInternalServerError)
			return
		}
	}
	w.WriteHeader(http.StatusAccepted)
	_, _ = w.Write([]byte("regeneration requested"))
}

// handlePush streams an SSE `content` event whenever content.json's mtime
// changes, so the page picks up driver regenerations without polling the
// document itself. Origin-checked loopback only (T-019).
func (s *server) handlePush(w http.ResponseWriter, r *http.Request) {
	if !guardLoopback(w, r) {
		return
	}
	fl, ok := w.(http.Flusher)
	if !ok {
		http.Error(w, "streaming unsupported", http.StatusInternalServerError)
		return
	}
	w.Header().Set("Content-Type", "text/event-stream")
	w.Header().Set("Cache-Control", "no-cache")
	w.Header().Set("X-Accel-Buffering", "no")
	_, _ = fmt.Fprint(w, "retry: 2000\n\n")
	fl.Flush()

	last := contentModTime(s.contentPath)
	ticker := time.NewTicker(500 * time.Millisecond)
	defer ticker.Stop()
	for {
		select {
		case <-r.Context().Done():
			return
		case <-ticker.C:
			cur := contentModTime(s.contentPath)
			if !cur.Equal(last) {
				last = cur
				_, _ = fmt.Fprint(w, "event: content\ndata: changed\n\n")
				fl.Flush()
			}
		}
	}
}

func contentModTime(path string) time.Time {
	info, err := os.Stat(path)
	if err != nil {
		return time.Time{}
	}
	return info.ModTime()
}

// guardLoopback enforces the loopback + same-origin gate shared by the
// mutation and push endpoints. Returns false after writing the rejection.
func guardLoopback(w http.ResponseWriter, r *http.Request) bool {
	if !requestIsLoopback(r) {
		http.Error(w, "forbidden: loopback only", http.StatusForbidden)
		return false
	}
	if origin := r.Header.Get("Origin"); origin != "" && !originIsLoopback(origin) {
		http.Error(w, "forbidden: cross-origin request", http.StatusForbidden)
		return false
	}
	return true
}

// requestIsLoopback reports whether the request arrived over loopback.
func requestIsLoopback(r *http.Request) bool {
	host, _, err := net.SplitHostPort(r.RemoteAddr)
	if err != nil {
		return false
	}
	return hostIsLoopback(host)
}

// originIsLoopback reports whether an Origin header points at loopback.
func originIsLoopback(origin string) bool {
	u, err := url.Parse(origin)
	if err != nil || u.Hostname() == "" {
		return false
	}
	return hostIsLoopback(u.Hostname())
}

func hostIsLoopback(host string) bool {
	if strings.EqualFold(host, "localhost") {
		return true
	}
	ip := net.ParseIP(host)
	return ip != nil && ip.IsLoopback()
}

// loopbackAddr reports whether addr is a bind address whose host part is an
// explicit loopback. An EMPTY host (":8090") binds every interface and is
// therefore rejected — the default is always the explicit "127.0.0.1:8090"
// (T-020: no accidental exposure of fabricated data).
func loopbackAddr(addr string) bool {
	host, _, err := net.SplitHostPort(addr)
	if err != nil || host == "" {
		return false
	}
	return hostIsLoopback(host)
}

func main() {
	addr := flag.String("addr", "127.0.0.1:8090",
		"loopback listen address; the host must be 127.0.0.1, ::1, or localhost")
	content := flag.String("content", "",
		"path to the driver's content.json (required)")
	tool := flag.String("tool", "virtual tool",
		"tool name shown in the page chrome")
	regen := flag.String("regen", "",
		"path to regen.request (default: <content dir>/regen.request)")
	flag.Parse()

	if *content == "" {
		fmt.Fprintln(os.Stderr, "web-vtool: --content is required (the driver prints the path)")
		os.Exit(2)
	}
	if *regen == "" {
		*regen = filepath.Join(filepath.Dir(*content), "regen.request")
	}
	if !loopbackAddr(*addr) {
		fmt.Fprintf(os.Stderr,
			"web-vtool: refusing to bind %q — this service serves fabricated data on loopback only; use 127.0.0.1:<port> (THREAT-MODEL T-020)\n",
			*addr)
		os.Exit(2)
	}
	if _, err := os.Stat(*content); errors.Is(err, os.ErrNotExist) {
		fmt.Fprintf(os.Stderr,
			"web-vtool: note: %s does not exist yet; the page will show \"content not ready\" until the driver generates it\n",
			*content)
	}

	srv := &server{contentPath: *content, regenPath: *regen, tool: *tool}
	fmt.Fprintf(os.Stderr,
		"web-vtool: serving %q at http://%s (loopback only; fabricated data; no providers, no network egress)\n",
		*tool, *addr)
	log.Fatal(http.ListenAndServe(*addr, srv.routes()))
}
