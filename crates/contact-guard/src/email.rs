use std::borrow::Cow;
use std::fmt::Write;

use crate::Message;

/// One line, safe to put in an email header: line breaks and other control characters
/// become spaces and runs of whitespace collapse. Borrows when nothing needed changing.
pub fn header_safe(s: &str) -> Cow<'_, str> {
    let clean = !s.chars().any(char::is_control) && !s.contains("  ") && s.trim() == s;
    if clean {
        return Cow::Borrowed(s);
    }
    let mut out = String::with_capacity(s.len());
    for word in s.split(|c: char| c.is_whitespace() || c.is_control()).filter(|w| !w.is_empty()) {
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(word);
    }
    Cow::Owned(out)
}

/// A plain-text email about a contact message, ready for SMTP `DATA`.
#[derive(Debug, Clone)]
pub struct PlainEmail<'a> {
    from: &'a str,
    to: &'a str,
    msg: &'a Message,
    subject_prefix: &'a str,
}

impl<'a> PlainEmail<'a> {
    /// From and to are your own addresses; reply-to is the visitor.
    pub fn new(from: &'a str, to: &'a str, msg: &'a Message) -> Self {
        PlainEmail { from, to, msg, subject_prefix: "Contact form message from" }
    }

    /// Change the words before the visitor's name in the subject.
    #[must_use]
    pub fn subject_prefix(mut self, prefix: &'a str) -> Self {
        self.subject_prefix = prefix;
        self
    }

    /// The subject line (unencoded).
    pub fn subject(&self) -> String {
        format!("{} {}", header_safe(self.subject_prefix), header_safe(self.msg.name()))
    }

    /// The whole message with CRLF line endings. `date` is an RFC 2822 date string.
    pub fn render(&self, date: &str) -> String {
        let mut out = String::with_capacity(self.msg.body().len() + 512);
        let name = header_safe(self.msg.name());
        let reply = header_safe(self.msg.email());
        let _ = write!(out, "From: {}\r\n", header_safe(self.from));
        let _ = write!(out, "To: {}\r\n", header_safe(self.to));
        let _ = write!(out, "Reply-To: {reply}\r\n");
        let _ = write!(out, "Subject: {}\r\n", encode_word(&self.subject()));
        let _ = write!(out, "Date: {}\r\n", header_safe(date));
        out.push_str("MIME-Version: 1.0\r\nContent-Type: text/plain; charset=UTF-8\r\nContent-Transfer-Encoding: 8bit\r\n\r\n");
        let _ = write!(out, "{name} <{reply}> wrote:\r\n\r\n");
        for line in self.msg.body().lines() {
            // a lone "." ends SMTP DATA early; dot-stuff it
            if line.starts_with('.') {
                out.push('.');
            }
            out.push_str(line);
            out.push_str("\r\n");
        }
        out
    }
}

/// RFC 2047 encoded-word for non-ASCII header text; ASCII passes through.
fn encode_word(s: &str) -> Cow<'_, str> {
    if s.is_ascii() {
        Cow::Borrowed(s)
    } else {
        Cow::Owned(format!("=?UTF-8?B?{}?=", base64(s.as_bytes())))
    }
}

fn base64(input: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    for chunk in input.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = u32::from(b[0]) << 16 | u32::from(b[1]) << 8 | u32::from(b[2]);
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(T[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Form, Submission, Verdict};

    fn msg(name: &str, body: &str) -> Message {
        // build directly: these tests probe the renderer, not the form rules
        Message { name: name.into(), email: "eve@example.com".into(), body: body.into() }
    }

    #[test]
    fn header_safe_borrows_clean_text() {
        assert!(matches!(header_safe("Rudo Moyo"), Cow::Borrowed(_)));
        assert_eq!(header_safe("Eve\r\nBcc: victim@example.com"), "Eve Bcc: victim@example.com");
        assert_eq!(header_safe("  a \t b  "), "a b");
    }

    #[test]
    fn nothing_typed_can_add_a_header() {
        let m = msg("Eve\r\nBcc: victim@example.com", "hi");
        let raw = PlainEmail::new("me@gmail.com", "me@gmail.com", &m).render("Wed, 26 Nov 2026 10:00:00 +0200");
        let (head, _) = raw.split_once("\r\n\r\n").unwrap();
        assert!(!head.lines().any(|l| l.starts_with("Bcc:")), "{head}");
        assert!(head.contains("Reply-To: eve@example.com"));
    }

    #[test]
    fn body_is_crlf_and_dot_stuffed() {
        let m = msg("A", "line 1\n.\nline 3");
        let raw = PlainEmail::new("a@b.co", "a@b.co", &m).render("d");
        assert!(raw.ends_with("line 1\r\n..\r\nline 3\r\n"));
    }

    #[test]
    fn non_ascii_subject_is_encoded() {
        let m = msg("Zoë", "x");
        let raw = PlainEmail::new("a@b.co", "a@b.co", &m).render("d");
        let subject = raw.lines().find(|l| l.starts_with("Subject:")).unwrap();
        assert!(subject.starts_with("Subject: =?UTF-8?B?") && subject.is_ascii(), "{subject}");
    }

    #[test]
    fn base64_known_values() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64("Zoë".as_bytes()), "Wm/Dqw==");
    }

    #[test]
    fn works_from_a_checked_submission() {
        let Verdict::Accept(m) = Form::contact().check(Submission::new("Rudo", "rudo@example.com", "Hello")) else { panic!() };
        let e = PlainEmail::new("me@gmail.com", "me@gmail.com", &m).subject_prefix("Portfolio message from");
        assert_eq!(e.subject(), "Portfolio message from Rudo");
    }
}
