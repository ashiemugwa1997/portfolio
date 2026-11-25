// portfolio api: profile json, contact form and the built react site
package main

import (
	"crypto/sha256"
	_ "embed"
	"encoding/hex"
	"encoding/json"
	"errors"
	"flag"
	"log"
	"net"
	"net/http"
	"net/mail"
	"os"
	"path/filepath"
	"strings"
	"sync"
	"time"
	"unicode/utf8"
)

//go:embed content/profile.json
var profileJSON []byte

const securityPolicy = "default-src 'self'; script-src 'self' 'wasm-unsafe-eval'; style-src 'self'; " +
	"img-src 'self' data:; connect-src 'self'; frame-ancestors 'none'; base-uri 'none'; form-action 'self'"

// ── Server ──────────────────────────────────────────────────────────────────

func main() {
	addr := flag.String("addr", ":8080", "listen address")
	static := flag.String("static", "../web/dist", "built React site to serve")
	data := flag.String("data", "./data", "folder for contact messages")
	flag.Parse()

	if err := os.MkdirAll(*data, 0o700); err != nil {
		log.Fatal(err)
	}
	srv := &http.Server{
		Addr:              *addr,
		Handler:           newServer(*static, filepath.Join(*data, "messages.jsonl"), time.Now).routes(),
		ReadHeaderTimeout: 5 * time.Second,
		ReadTimeout:       10 * time.Second,
		WriteTimeout:      15 * time.Second,
	}
	log.Printf("portfolio on %s (site %s)", *addr, *static)
	log.Fatal(srv.ListenAndServe())
}

type server struct {
	static   string
	inbox    string
	now      func() time.Time
	etag     string
	limiter  *limiter
	writeMux sync.Mutex
}

func newServer(static, inbox string, now func() time.Time) *server {
	sum := sha256.Sum256(profileJSON)
	return &server{
		static:  static,
		inbox:   inbox,
		now:     now,
		etag:    `"` + hex.EncodeToString(sum[:8]) + `"`,
		limiter: newLimiter(3, time.Hour, now),
	}
}

func (s *server) routes() http.Handler {
	mux := http.NewServeMux()
	mux.HandleFunc("GET /healthz", func(w http.ResponseWriter, _ *http.Request) { w.Write([]byte("ok")) })
	mux.HandleFunc("GET /api/profile", s.profile)
	mux.HandleFunc("POST /api/contact", s.contact)
	mux.HandleFunc("/api/", func(w http.ResponseWriter, _ *http.Request) { writeJSON(w, 404, map[string]string{"error": "not found"}) })
	mux.HandleFunc("GET /", s.site)
	return secure(mux)
}

func secure(next http.Handler) http.Handler {
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		h := w.Header()
		h.Set("Content-Security-Policy", securityPolicy)
		h.Set("X-Content-Type-Options", "nosniff")
		h.Set("Referrer-Policy", "strict-origin-when-cross-origin")
		h.Set("Permissions-Policy", "camera=(), microphone=(), geolocation=()")
		next.ServeHTTP(w, r)
	})
}

// ── Profile ─────────────────────────────────────────────────────────────────

func (s *server) profile(w http.ResponseWriter, r *http.Request) {
	w.Header().Set("ETag", s.etag)
	w.Header().Set("Cache-Control", "no-cache")
	if r.Header.Get("If-None-Match") == s.etag {
		w.WriteHeader(http.StatusNotModified)
		return
	}
	w.Header().Set("Content-Type", "application/json")
	w.Write(profileJSON)
}

// ── Contact form ────────────────────────────────────────────────────────────

type message struct {
	Name     string `json:"name"`
	Email    string `json:"email"`
	Message  string `json:"message"`
	Website  string `json:"website"` // honeypot, hidden from people
	Received string `json:"received,omitempty"`
}

func (m *message) validate() error {
	m.Name, m.Email, m.Message = strings.TrimSpace(m.Name), strings.TrimSpace(m.Email), strings.TrimSpace(m.Message)
	switch {
	case m.Name == "" || utf8.RuneCountInString(m.Name) > 100:
		return errors.New("Please give your name (up to 100 characters).")
	case len(m.Email) > 200:
		return errors.New("That email address is too long.")
	case m.Message == "" || utf8.RuneCountInString(m.Message) > 4000:
		return errors.New("Please write a message (up to 4,000 characters).")
	}
	if a, err := mail.ParseAddress(m.Email); err != nil || a.Address != m.Email || !strings.Contains(m.Email, ".") {
		return errors.New("Please check your email address.")
	}
	return nil
}

func (s *server) contact(w http.ResponseWriter, r *http.Request) {
	if !strings.HasPrefix(r.Header.Get("Content-Type"), "application/json") {
		writeJSON(w, 415, map[string]string{"error": "Send JSON."})
		return
	}
	var m message
	dec := json.NewDecoder(http.MaxBytesReader(w, r.Body, 16<<10))
	dec.DisallowUnknownFields()
	if err := dec.Decode(&m); err != nil {
		writeJSON(w, 400, map[string]string{"error": "That message could not be read."})
		return
	}
	// bot filled the hidden field, say ok and drop it
	if m.Website != "" {
		writeJSON(w, 202, map[string]string{"status": "received"})
		return
	}
	if err := m.validate(); err != nil {
		writeJSON(w, 422, map[string]string{"error": err.Error()})
		return
	}
	if !s.limiter.allow(clientIP(r)) {
		writeJSON(w, 429, map[string]string{"error": "Thanks, I have your messages. Please try again later."})
		return
	}
	m.Website, m.Received = "", s.now().UTC().Format(time.RFC3339)
	if err := s.store(m); err != nil {
		log.Printf("contact store: %v", err)
		writeJSON(w, 500, map[string]string{"error": "Your message could not be saved. Please email me instead."})
		return
	}
	writeJSON(w, 202, map[string]string{"status": "received"})
}

func (s *server) store(m message) error {
	line, err := json.Marshal(m)
	if err != nil {
		return err
	}
	s.writeMux.Lock()
	defer s.writeMux.Unlock()
	f, err := os.OpenFile(s.inbox, os.O_CREATE|os.O_APPEND|os.O_WRONLY, 0o600)
	if err != nil {
		return err
	}
	defer f.Close()
	_, err = f.Write(append(line, '\n'))
	return err
}

// ── Static site ─────────────────────────────────────────────────────────────

// files from web/dist, anything else gets index.html
func (s *server) site(w http.ResponseWriter, r *http.Request) {
	root, err := filepath.Abs(s.static)
	if err != nil {
		http.Error(w, "site not built", 500)
		return
	}
	clean := filepath.Clean("/" + r.URL.Path)
	path := filepath.Join(root, filepath.FromSlash(clean))
	if !strings.HasPrefix(path, root) {
		http.NotFound(w, r)
		return
	}
	if info, err := os.Stat(path); err == nil && !info.IsDir() {
		if strings.HasPrefix(clean, "/assets/") {
			w.Header().Set("Cache-Control", "public, max-age=31536000, immutable")
		}
		http.ServeFile(w, r, path)
		return
	}
	index := filepath.Join(root, "index.html")
	if _, err := os.Stat(index); err != nil {
		http.Error(w, "The site is not built yet: run npm run build in web/.", 503)
		return
	}
	w.Header().Set("Cache-Control", "no-cache")
	http.ServeFile(w, r, index)
}

// ── Helpers ─────────────────────────────────────────────────────────────────

func clientIP(r *http.Request) string {
	host, _, err := net.SplitHostPort(r.RemoteAddr)
	if err != nil {
		return r.RemoteAddr
	}
	return host
}

func writeJSON(w http.ResponseWriter, status int, v any) {
	w.Header().Set("Content-Type", "application/json")
	w.WriteHeader(status)
	json.NewEncoder(w).Encode(v)
}

// n messages per window per address
type limiter struct {
	mu     sync.Mutex
	n      int
	window time.Duration
	now    func() time.Time
	seen   map[string][]time.Time
}

func newLimiter(n int, window time.Duration, now func() time.Time) *limiter {
	return &limiter{n: n, window: window, now: now, seen: map[string][]time.Time{}}
}

func (l *limiter) allow(key string) bool {
	l.mu.Lock()
	defer l.mu.Unlock()
	cutoff := l.now().Add(-l.window)
	kept := l.seen[key][:0]
	for _, t := range l.seen[key] {
		if t.After(cutoff) {
			kept = append(kept, t)
		}
	}
	if len(kept) >= l.n {
		l.seen[key] = kept
		return false
	}
	l.seen[key] = append(kept, l.now())
	return true
}
