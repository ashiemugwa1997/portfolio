package main

import (
	"bufio"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"
)

type clock struct{ t time.Time }

func (c *clock) now() time.Time { return c.t }

func setup(t *testing.T) (*server, http.Handler, *clock) {
	t.Helper()
	dir := t.TempDir()
	site := filepath.Join(dir, "dist")
	os.MkdirAll(filepath.Join(site, "assets"), 0o755)
	os.WriteFile(filepath.Join(site, "index.html"), []byte("<!doctype html><title>home</title>"), 0o644)
	os.WriteFile(filepath.Join(site, "assets", "app.js"), []byte("console.log(1)"), 0o644)
	c := &clock{t: time.Date(2026, 11, 25, 10, 0, 0, 0, time.UTC)}
	s := newServer(site, filepath.Join(dir, "messages.jsonl"), c.now)
	return s, s.routes(), c
}

func do(h http.Handler, method, path, body string, hdr map[string]string) *httptest.ResponseRecorder {
	r := httptest.NewRequest(method, path, strings.NewReader(body))
	r.RemoteAddr = "10.0.0.1:5000"
	for k, v := range hdr {
		r.Header.Set(k, v)
	}
	w := httptest.NewRecorder()
	h.ServeHTTP(w, r)
	return w
}

var jsonHdr = map[string]string{"Content-Type": "application/json"}

func TestProfileIsValidJSONWithEtag(t *testing.T) {
	_, h, _ := setup(t)
	w := do(h, "GET", "/api/profile", "", nil)
	if w.Code != 200 {
		t.Fatalf("status %d", w.Code)
	}
	var p struct {
		Name    string `json:"name"`
		Metrics []any  `json:"metrics"`
	}
	if err := json.Unmarshal(w.Body.Bytes(), &p); err != nil || p.Name == "" || len(p.Metrics) == 0 {
		t.Fatalf("profile: %v %+v", err, p)
	}
	etag := w.Header().Get("ETag")
	if w2 := do(h, "GET", "/api/profile", "", map[string]string{"If-None-Match": etag}); w2.Code != 304 {
		t.Fatalf("want 304, got %d", w2.Code)
	}
}

func TestEveryResponseCarriesSecurityHeaders(t *testing.T) {
	_, h, _ := setup(t)
	for _, p := range []string{"/", "/api/profile", "/healthz"} {
		w := do(h, "GET", p, "", nil)
		if !strings.Contains(w.Header().Get("Content-Security-Policy"), "frame-ancestors 'none'") {
			t.Errorf("%s: no CSP", p)
		}
		if w.Header().Get("X-Content-Type-Options") != "nosniff" {
			t.Errorf("%s: no nosniff", p)
		}
	}
}

func TestContactStoresValidMessage(t *testing.T) {
	s, h, _ := setup(t)
	w := do(h, "POST", "/api/contact", `{"name":" Rudo ","email":"rudo@example.com","message":"Hello"}`, jsonHdr)
	if w.Code != 202 {
		t.Fatalf("status %d %s", w.Code, w.Body)
	}
	f, _ := os.Open(s.inbox)
	defer f.Close()
	sc := bufio.NewScanner(f)
	if !sc.Scan() {
		t.Fatal("nothing stored")
	}
	var m message
	json.Unmarshal(sc.Bytes(), &m)
	if m.Name != "Rudo" || m.Received != "2026-11-25T10:00:00Z" {
		t.Fatalf("stored %+v", m)
	}
}

func TestContactRejectsBadInput(t *testing.T) {
	_, h, _ := setup(t)
	cases := map[string]struct {
		body string
		code int
	}{
		"no name":       {`{"name":"","email":"a@b.co","message":"x"}`, 422},
		"bad email":     {`{"name":"A","email":"not-an-email","message":"x"}`, 422},
		"display email": {`{"name":"A","email":"A <a@b.co>","message":"x"}`, 422},
		"empty message": {`{"name":"A","email":"a@b.co","message":"   "}`, 422},
		"long message":  {`{"name":"A","email":"a@b.co","message":"` + strings.Repeat("x", 4001) + `"}`, 422},
		"unknown field": {`{"name":"A","email":"a@b.co","message":"x","admin":true}`, 400},
		"not json":      {`name=A`, 400},
	}
	for name, c := range cases {
		if w := do(h, "POST", "/api/contact", c.body, jsonHdr); w.Code != c.code {
			t.Errorf("%s: want %d, got %d", name, c.code, w.Code)
		}
	}
	if w := do(h, "POST", "/api/contact", `{}`, map[string]string{"Content-Type": "text/plain"}); w.Code != 415 {
		t.Errorf("text/plain: want 415, got %d", w.Code)
	}
}

func TestHoneypotIsAcceptedButNotStored(t *testing.T) {
	s, h, _ := setup(t)
	w := do(h, "POST", "/api/contact", `{"name":"Bot","email":"b@b.co","message":"buy","website":"x"}`, jsonHdr)
	if w.Code != 202 {
		t.Fatalf("status %d", w.Code)
	}
	if _, err := os.Stat(s.inbox); !os.IsNotExist(err) {
		t.Fatal("honeypot message was stored")
	}
}

func TestContactIsRateLimitedPerAddress(t *testing.T) {
	_, h, c := setup(t)
	body := `{"name":"A","email":"a@b.co","message":"hi"}`
	for i := 0; i < 3; i++ {
		if w := do(h, "POST", "/api/contact", body, jsonHdr); w.Code != 202 {
			t.Fatalf("message %d: %d", i, w.Code)
		}
	}
	if w := do(h, "POST", "/api/contact", body, jsonHdr); w.Code != 429 {
		t.Fatalf("4th: want 429, got %d", w.Code)
	}
	c.t = c.t.Add(61 * time.Minute)
	if w := do(h, "POST", "/api/contact", body, jsonHdr); w.Code != 202 {
		t.Fatalf("after the window: %d", w.Code)
	}
}

func TestSiteServesFilesAndFallsBackToIndex(t *testing.T) {
	_, h, _ := setup(t)
	if w := do(h, "GET", "/assets/app.js", "", nil); w.Code != 200 || !strings.Contains(w.Header().Get("Cache-Control"), "immutable") {
		t.Fatalf("asset: %d %q", w.Code, w.Header().Get("Cache-Control"))
	}
	// mux cleans ../ itself with a redirect to the clean path
	if w := do(h, "GET", "/../../etc/passwd", "", nil); w.Code != 301 || w.Header().Get("Location") != "/etc/passwd" {
		t.Errorf("traversal: %d %q", w.Code, w.Header().Get("Location"))
	}
	for _, p := range []string{"/", "/experience", "/etc/passwd", "/main.go"} {
		w := do(h, "GET", p, "", nil)
		if w.Code != 200 || !strings.Contains(w.Body.String(), "<title>home</title>") {
			t.Errorf("%s: %d %q", p, w.Code, w.Body.String())
		}
	}
	if w := do(h, "GET", "/api/nothing", "", nil); w.Code != 404 {
		t.Errorf("unknown api: %d", w.Code)
	}
}
