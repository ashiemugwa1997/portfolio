package main

import (
	"bufio"
	"fmt"
	"log"
	"net/smtp"
	"os"
	"strings"
	"time"
)

// ── .env ────────────────────────────────────────────────────────────────────

// loadEnv reads KEY=value lines; real environment variables win
func loadEnv(file string) {
	f, err := os.Open(file)
	if err != nil {
		return
	}
	defer f.Close()
	sc := bufio.NewScanner(f)
	for sc.Scan() {
		line := strings.TrimSpace(sc.Text())
		if line == "" || strings.HasPrefix(line, "#") {
			continue
		}
		key, val, ok := strings.Cut(line, "=")
		if !ok {
			continue
		}
		key = strings.TrimSpace(key)
		val = strings.Trim(strings.TrimSpace(val), `"'`)
		if _, set := os.LookupEnv(key); !set {
			os.Setenv(key, val)
		}
	}
}

// ── Mailer ──────────────────────────────────────────────────────────────────

type mailer interface {
	send(m message) error
}

// gmail sends through smtp.gmail.com with an app password
type gmail struct {
	user, pass, to, host, port string
}

// mailerFromEnv gives nil when SMTP_USER or SMTP_PASS is empty: messages then go to the file only
func mailerFromEnv() mailer {
	g := &gmail{
		user: os.Getenv("SMTP_USER"),
		pass: strings.ReplaceAll(os.Getenv("SMTP_PASS"), " ", ""), // google shows it in groups of 4
		to:   os.Getenv("MAIL_TO"),
		host: envOr("SMTP_HOST", "smtp.gmail.com"),
		port: envOr("SMTP_PORT", "587"),
	}
	if g.user == "" || g.pass == "" {
		return nil
	}
	if g.to == "" {
		g.to = g.user
	}
	return g
}

func (g *gmail) send(m message) error {
	auth := smtp.PlainAuth("", g.user, g.pass, g.host)
	return smtp.SendMail(g.host+":"+g.port, auth, g.user, []string{g.to}, compose(g.user, g.to, m))
}

// compose builds the email; reply-to is the visitor so Reply answers them
func compose(from, to string, m message) []byte {
	name := oneLine(m.Name)
	var b strings.Builder
	fmt.Fprintf(&b, "From: Portfolio <%s>\r\n", from)
	fmt.Fprintf(&b, "To: %s\r\n", to)
	fmt.Fprintf(&b, "Reply-To: %s\r\n", oneLine(m.Email))
	fmt.Fprintf(&b, "Subject: Portfolio message from %s\r\n", name)
	fmt.Fprintf(&b, "Date: %s\r\n", time.Now().Format(time.RFC1123Z))
	b.WriteString("MIME-Version: 1.0\r\nContent-Type: text/plain; charset=UTF-8\r\n\r\n")
	fmt.Fprintf(&b, "%s <%s> wrote:\r\n\r\n", name, oneLine(m.Email))
	b.WriteString(strings.ReplaceAll(strings.ReplaceAll(m.Message, "\r\n", "\n"), "\n", "\r\n"))
	b.WriteString("\r\n")
	return []byte(b.String())
}

// oneLine strips line breaks so nothing can add its own email headers
func oneLine(s string) string {
	return strings.Join(strings.Fields(strings.NewReplacer("\r", " ", "\n", " ").Replace(s)), " ")
}

func envOr(key, def string) string {
	if v := os.Getenv(key); v != "" {
		return v
	}
	return def
}

// notify emails in the background; the file copy is already saved
func (s *server) notify(m message) {
	if s.mail == nil {
		return
	}
	go func() {
		if err := s.mail.send(m); err != nil {
			log.Printf("email: %v", err)
		}
	}()
}
