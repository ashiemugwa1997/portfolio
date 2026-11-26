//! Hostile input against the form and the email renderer.
use contact_guard::{header_safe, Field, Form, PlainEmail, Submission, Verdict};
use proptest::prelude::*;

fn nasty() -> impl Strategy<Value = String> {
    prop_oneof![
        3 => any::<String>(),
        2 => "[a-zA-Z .'\r\n\t\u{0}\u{1b}\u{202e}:@<>\"]{0,60}",
        1 => Just("Eve\r\nBcc: victim@example.com".to_string()),
        1 => Just(".\r\n.\r\nQUIT".to_string()),
        1 => "[a-z]{1,10}@[a-z]{1,10}\\.[a-z]{2,4}",
    ]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(8192))]

    #[test]
    fn accepted_messages_keep_every_promise(name in nasty(), email in nasty(), message in nasty(), honeypot in prop_oneof![Just(String::new()), nasty()]) {
        let sub = Submission { name, email, message, honeypot: honeypot.clone() };
        match Form::contact().check(sub) {
            Verdict::Bot => prop_assert!(!honeypot.trim().is_empty()),
            Verdict::Reject(p) => prop_assert!(!p.is_empty()),
            Verdict::Accept(m) => {
                prop_assert!(honeypot.trim().is_empty());
                prop_assert!(!m.name().is_empty() && m.name().chars().count() <= 100);
                prop_assert!(!m.name().chars().any(char::is_control));
                prop_assert!(m.email().len() <= 200 && m.email().contains('@') && !m.email().chars().any(|c| c.is_whitespace() || c.is_control()));
                prop_assert!(!m.body().is_empty() && m.body().chars().count() <= 4000);
                prop_assert_eq!(m.name().trim(), m.name());
            }
        }
    }

    #[test]
    fn rendered_email_cannot_grow_headers(name in "[^\u{0}]{1,40}", body in any::<String>()) {
        // build an accepted message straight from the form when possible
        let sub = Submission::new(name, "visitor@example.com", if body.trim().is_empty() { "x".into() } else { body });
        if let Verdict::Accept(m) = Form::contact().check(sub) {
            let raw = PlainEmail::new("me@example.com", "me@example.com", &m).render("Wed, 26 Nov 2026 10:00:00 +0200");
            let (head, rest) = raw.split_once("\r\n\r\n").expect("blank line ends the headers");
            let names: Vec<&str> = head.split("\r\n").map(|l| l.split(':').next().unwrap_or("")).collect();
            prop_assert_eq!(names, vec!["From", "To", "Reply-To", "Subject", "Date", "MIME-Version", "Content-Type", "Content-Transfer-Encoding"]);
            prop_assert!(head.is_ascii() || !head.lines().any(|l| l.starts_with("Subject:") && !l.is_ascii()));
            // no bare LF or CR in the body, and no line that is a lone dot
            prop_assert!(!rest.replace("\r\n", "").contains(['\r', '\n']));
            prop_assert!(!rest.split("\r\n").any(|l| l == "."));
        }
    }

    #[test]
    fn header_safe_is_one_clean_line_and_idempotent(s in any::<String>()) {
        let once = header_safe(&s).into_owned();
        prop_assert!(!once.chars().any(char::is_control));
        prop_assert_eq!(once.trim(), once.as_str());
        prop_assert!(!once.contains("  "));
        prop_assert_eq!(header_safe(&once).into_owned(), once);
    }

    #[test]
    fn problems_name_only_real_fields(name in nasty(), email in nasty(), message in nasty()) {
        if let Verdict::Reject(p) = Form::contact().check(Submission::new(name, email, message)) {
            prop_assert!(p.iter().all(|(f, msg)| Field::ALL.contains(&f) && !msg.is_empty()));
        }
    }
}
