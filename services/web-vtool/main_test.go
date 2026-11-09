package main

import (
	"bufio"
	"context"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"
)

func newServer(contentPath, regenPath string) *server {
	return &server{contentPath: contentPath, regenPath: regenPath, tool: "mock-page"}
}

func doGet(t *testing.T, h http.Handler, url string) *httptest.ResponseRecorder {
	t.Helper()
	req := httptest.NewRequest(http.MethodGet, url, nil)
	rec := httptest.NewRecorder()
	h.ServeHTTP(rec, req)
	return rec
}

func writeContent(t *testing.T, path, body string) {
	t.Helper()
	if err := os.WriteFile(path, []byte(body), 0o600); err != nil {
		t.Fatal(err)
	}
}

// T-019: fabricated strings must render escaped, never as markup.
func TestIndexEscapesFabricatedHTML(t *testing.T) {
	dir := t.TempDir()
	content := filepath.Join(dir, "content.json")
	writeContent(t, content,
		`{"title":"<script>alert(1)</script>","body":"<img src=x onerror=alert(2)>","note":"plain & simple"}`)
	rec := doGet(t, newServer(content, filepath.Join(dir, "regen.request")).routes(), "/")
	if rec.Code != http.StatusOK {
		t.Fatalf("status %d", rec.Code)
	}
	body := rec.Body.String()
	if strings.Contains(body, "<script>alert") {
		t.Fatalf("raw <script> reached the page:\n%s", body)
	}
	if strings.Contains(body, "<img src=x") {
		t.Fatalf("raw <img> reached the page:\n%s", body)
	}
	if !strings.Contains(body, "&lt;script&gt;alert(1)&lt;/script&gt;") {
		t.Fatalf("escaped title missing:\n%s", body)
	}
	if !strings.Contains(body, "plain &amp; simple") {
		t.Fatalf("escaped ampersand missing:\n%s", body)
	}
}

func TestIndexShowsNotReadyWhenContentMissing(t *testing.T) {
	dir := t.TempDir()
	rec := doGet(t, newServer(filepath.Join(dir, "content.json"), filepath.Join(dir, "regen.request")).routes(), "/")
	if rec.Code != http.StatusOK {
		t.Fatalf("status %d", rec.Code)
	}
	if !strings.Contains(rec.Body.String(), "content not ready") {
		t.Fatalf("missing not-ready notice:\n%s", rec.Body.String())
	}
}

func TestContentEndpointServesRawJSON(t *testing.T) {
	dir := t.TempDir()
	content := filepath.Join(dir, "content.json")
	writeContent(t, content, `{"title":"raw"}`)
	rec := doGet(t, newServer(content, filepath.Join(dir, "regen.request")).routes(), "/content.json")
	if rec.Code != http.StatusOK {
		t.Fatalf("status %d", rec.Code)
	}
	if got := rec.Header().Get("Content-Type"); got != "application/json" {
		t.Fatalf("content type %q", got)
	}
	if rec.Body.String() != `{"title":"raw"}` {
		t.Fatalf("body %q", rec.Body.String())
	}
}

// T-019: /regen can spend the driver's model-turn budget — loopback only.
func TestRegenRequiresLoopbackRemoteAddr(t *testing.T) {
	dir := t.TempDir()
	regen := filepath.Join(dir, "regen.request")
	h := newServer(filepath.Join(dir, "content.json"), regen).routes()

	req := httptest.NewRequest(http.MethodPost, "/regen", nil)
	req.RemoteAddr = "10.1.2.3:5000"
	rec := httptest.NewRecorder()
	h.ServeHTTP(rec, req)
	if rec.Code != http.StatusForbidden {
		t.Fatalf("status %d, want 403", rec.Code)
	}
	if _, err := os.Stat(regen); !os.IsNotExist(err) {
		t.Fatal("regen.request was created by a non-loopback request")
	}
}

// T-019: a drive-by web page must not be able to fire regen even from a
// loopback browser (cross-origin POST).
func TestRegenRejectsCrossOrigin(t *testing.T) {
	dir := t.TempDir()
	regen := filepath.Join(dir, "regen.request")
	h := newServer(filepath.Join(dir, "content.json"), regen).routes()

	req := httptest.NewRequest(http.MethodPost, "/regen", nil)
	req.RemoteAddr = "127.0.0.1:5000"
	req.Header.Set("Origin", "https://evil.example")
	rec := httptest.NewRecorder()
	h.ServeHTTP(rec, req)
	if rec.Code != http.StatusForbidden {
		t.Fatalf("status %d, want 403", rec.Code)
	}
	if _, err := os.Stat(regen); !os.IsNotExist(err) {
		t.Fatal("regen.request was created by a cross-origin request")
	}
}

func TestRegenTouchesRequestFile(t *testing.T) {
	dir := t.TempDir()
	content := filepath.Join(dir, "content.json")
	regen := filepath.Join(dir, "regen.request")
	h := newServer(content, regen).routes()

	req := httptest.NewRequest(http.MethodPost, "/regen", nil)
	req.RemoteAddr = "127.0.0.1:5000"
	rec := httptest.NewRecorder()
	h.ServeHTTP(rec, req)
	if rec.Code != http.StatusAccepted {
		t.Fatalf("status %d, want 202", rec.Code)
	}
	if _, err := os.Stat(regen); err != nil {
		t.Fatalf("regen.request not created: %v", err)
	}

	// Pre-existing file: the mtime must move forward (the driver's watch
	// loop keys off mtime changes).
	old := time.Now().Add(-time.Hour)
	if err := os.Chtimes(regen, old, old); err != nil {
		t.Fatal(err)
	}
	rec = httptest.NewRecorder()
	h.ServeHTTP(rec, req)
	if rec.Code != http.StatusAccepted {
		t.Fatalf("second status %d", rec.Code)
	}
	info, err := os.Stat(regen)
	if err != nil {
		t.Fatal(err)
	}
	if info.ModTime().Before(time.Now().Add(-time.Minute)) {
		t.Fatalf("mtime not refreshed: %v", info.ModTime())
	}
}

func TestRegenRejectsGetMethod(t *testing.T) {
	dir := t.TempDir()
	req := httptest.NewRequest(http.MethodGet, "/regen", nil)
	req.RemoteAddr = "127.0.0.1:5000"
	rec := httptest.NewRecorder()
	newServer(filepath.Join(dir, "content.json"), filepath.Join(dir, "regen.request")).routes().ServeHTTP(rec, req)
	if rec.Code != http.StatusMethodNotAllowed {
		t.Fatalf("status %d, want 405", rec.Code)
	}
}

// T-019: the push channel is loopback + same-origin too.
func TestPushRejectsNonLoopbackAndCrossOrigin(t *testing.T) {
	dir := t.TempDir()
	h := newServer(filepath.Join(dir, "content.json"), filepath.Join(dir, "regen.request")).routes()

	req := httptest.NewRequest(http.MethodGet, "/push", nil)
	req.RemoteAddr = "10.1.2.3:5000"
	rec := httptest.NewRecorder()
	h.ServeHTTP(rec, req)
	if rec.Code != http.StatusForbidden {
		t.Fatalf("remote-addr status %d, want 403", rec.Code)
	}

	req = httptest.NewRequest(http.MethodGet, "/push", nil)
	req.RemoteAddr = "127.0.0.1:5000"
	req.Header.Set("Origin", "https://evil.example")
	rec = httptest.NewRecorder()
	h.ServeHTTP(rec, req)
	if rec.Code != http.StatusForbidden {
		t.Fatalf("origin status %d, want 403", rec.Code)
	}
}

// T-020: the bind-address gate refuses anything but explicit loopback.
func TestLoopbackAddrValidation(t *testing.T) {
	cases := map[string]bool{
		"127.0.0.1:8090": true,
		"127.5.5.5:80":   true,
		"localhost:80":   true,
		"LOCALHOST:80":   true,
		"[::1]:8090":     true,
		":8090":          false, // empty host binds every interface
		"0.0.0.0:8090":   false,
		"10.0.0.1:8090":  false,
		"noizu.com:80":   false,
		"127.0.0.1":      false, // no port
		"[fe80::1]:8090": false,
		"::1":            false, // no port
	}
	for addr, want := range cases {
		if got := loopbackAddr(addr); got != want {
			t.Errorf("loopbackAddr(%q) = %v, want %v", addr, got, want)
		}
	}
}

// End-to-end: an SSE client receives a `content` event when content.json's
// mtime moves — the signal a driver regeneration sends.
func TestPushStreamsContentEvent(t *testing.T) {
	dir := t.TempDir()
	content := filepath.Join(dir, "content.json")
	writeContent(t, content, `{"title":"before"}`)
	s := newServer(content, filepath.Join(dir, "regen.request"))
	ts := httptest.NewServer(s.routes())
	defer ts.Close()

	ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
	defer cancel()
	req, err := http.NewRequestWithContext(ctx, http.MethodGet, ts.URL+"/push", nil)
	if err != nil {
		t.Fatal(err)
	}
	resp, err := ts.Client().Do(req)
	if err != nil {
		t.Fatal(err)
	}
	defer resp.Body.Close()
	if got := resp.Header.Get("Content-Type"); got != "text/event-stream" {
		t.Fatalf("content type %q", got)
	}

	events := make(chan string, 4)
	go func() {
		sc := bufio.NewScanner(resp.Body)
		for sc.Scan() {
			line := sc.Text()
			if strings.HasPrefix(line, "event:") {
				events <- line
				return
			}
		}
	}()

	// Move content.json's mtime forward, exactly like a driver regen write.
	future := time.Now().Add(2 * time.Second)
	if err := os.Chtimes(content, future, future); err != nil {
		t.Fatal(err)
	}

	select {
	case ev := <-events:
		if !strings.Contains(ev, "content") {
			t.Fatalf("unexpected event %q", ev)
		}
	case <-time.After(5 * time.Second):
		t.Fatal("no content event within 5s")
	case <-ctx.Done():
		t.Fatal("test context expired")
	}
}
