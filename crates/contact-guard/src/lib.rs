//! Contact forms without the usual holes.
//!
//! * [`Form`] checks a [`Submission`] field by field and reports every problem at once.
//! * A honeypot field catches bots without a captcha.
//! * [`PlainEmail`] turns the result into an RFC 5322 message where nothing the visitor
//!   typed can add headers (no `Bcc:` smuggled in through the name field).
//!
//! ```
//! use contact_guard::{Form, Submission, Verdict};
//!
//! let form = Form::contact();
//! let sub = Submission::new("Rudo", "rudo@example.com", "Hello!");
//! match form.check(sub) {
//!     Verdict::Accept(msg) => assert_eq!(msg.name(), "Rudo"),
//!     Verdict::Reject(problems) => panic!("{problems}"),
//!     Verdict::Bot => unreachable!(),
//! }
//! ```
#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod email;
mod rules;

use std::collections::BTreeMap;
use std::fmt;

pub use email::{header_safe, PlainEmail};
pub use rules::{EmailShape, FieldRule, MaxChars, NoControl, Required};

// ── Submission ──────────────────────────────────────────────────────────────

/// What the visitor sent. Trimmed by [`Form::check`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(deny_unknown_fields))]
pub struct Submission {
    /// Visitor's name.
    pub name: String,
    /// Visitor's email address.
    pub email: String,
    /// The message.
    pub message: String,
    /// The hidden field people never see. Anything in it means a bot.
    #[cfg_attr(feature = "serde", serde(default, alias = "website"))]
    pub honeypot: String,
}

impl Submission {
    /// A submission with an empty honeypot.
    pub fn new(name: impl Into<String>, email: impl Into<String>, message: impl Into<String>) -> Self {
        Submission { name: name.into(), email: email.into(), message: message.into(), honeypot: String::new() }
    }

    fn get(&self, f: Field) -> &str {
        match f {
            Field::Name => &self.name,
            Field::Email => &self.email,
            Field::Message => &self.message,
        }
    }
}

/// The form's fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Field {
    /// Name.
    Name,
    /// Email address.
    Email,
    /// Message.
    Message,
}

impl Field {
    /// All fields in display order.
    pub const ALL: [Field; 3] = [Field::Name, Field::Email, Field::Message];

    /// Lowercase field name, as used in JSON.
    pub fn as_str(self) -> &'static str {
        match self {
            Field::Name => "name",
            Field::Email => "email",
            Field::Message => "message",
        }
    }
}

// ── Outcome ─────────────────────────────────────────────────────────────────

/// A submission that passed every rule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    name: String,
    email: String,
    body: String,
}

impl Message {
    /// Trimmed name.
    pub fn name(&self) -> &str {
        &self.name
    }
    /// Trimmed, checked email address.
    pub fn email(&self) -> &str {
        &self.email
    }
    /// Trimmed message.
    pub fn body(&self) -> &str {
        &self.body
    }
}

/// Every problem found, at most one per field, in field order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Problems(BTreeMap<Field, &'static str>);

impl Problems {
    /// The message for one field.
    pub fn get(&self, f: Field) -> Option<&'static str> {
        self.0.get(&f).copied()
    }
    /// The first problem in field order, for a single-line error.
    pub fn first(&self) -> Option<(Field, &'static str)> {
        self.0.iter().next().map(|(f, m)| (*f, *m))
    }
    /// All problems in field order.
    pub fn iter(&self) -> impl Iterator<Item = (Field, &'static str)> + '_ {
        self.0.iter().map(|(f, m)| (*f, *m))
    }
    /// Number of fields with a problem.
    pub fn len(&self) -> usize {
        self.0.len()
    }
    /// True when nothing is wrong.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl fmt::Display for Problems {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut first = true;
        for (field, msg) in self.iter() {
            if !first {
                f.write_str("; ")?;
            }
            write!(f, "{}: {msg}", field.as_str())?;
            first = false;
        }
        Ok(())
    }
}

/// The result of checking a submission.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// Good to store or send.
    Accept(Message),
    /// The visitor needs to fix something.
    Reject(Problems),
    /// The honeypot was filled: answer as if accepted, keep nothing.
    Bot,
}

// ── Form ────────────────────────────────────────────────────────────────────

/// A set of rules per field. Rules run in order; the first that fails names the problem.
pub struct Form {
    rules: BTreeMap<Field, Vec<Box<dyn FieldRule>>>,
}

impl fmt::Debug for Form {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let counts: Vec<(Field, usize)> = self.rules.iter().map(|(k, v)| (*k, v.len())).collect();
        f.debug_struct("Form").field("rules", &counts).finish()
    }
}

impl Form {
    /// A form with no rules.
    pub fn empty() -> Self {
        Form { rules: BTreeMap::new() }
    }

    /// Sensible limits for a contact form: name up to 100 characters with no control
    /// characters, a real-looking email up to 200, a message up to 4,000.
    pub fn contact() -> Self {
        Form::empty()
            .rule(Field::Name, Required("Please give your name."))
            .rule(Field::Name, MaxChars(100, "Your name can be up to 100 characters."))
            .rule(Field::Name, NoControl("Your name has characters that are not allowed."))
            .rule(Field::Email, Required("Please give your email address."))
            .rule(Field::Email, MaxChars(200, "That email address is too long."))
            .rule(Field::Email, EmailShape("Please check your email address."))
            .rule(Field::Message, Required("Please write a message."))
            .rule(Field::Message, MaxChars(4000, "Your message can be up to 4,000 characters."))
    }

    /// Add a rule for `field`.
    #[must_use]
    pub fn rule(mut self, field: Field, rule: impl FieldRule + 'static) -> Self {
        self.rules.entry(field).or_default().push(Box::new(rule));
        self
    }

    /// Check a submission.
    pub fn check(&self, sub: Submission) -> Verdict {
        if !sub.honeypot.trim().is_empty() {
            return Verdict::Bot;
        }
        let mut problems = Problems::default();
        for field in Field::ALL {
            let value = sub.get(field).trim();
            let failed = self.rules.get(&field).into_iter().flatten().find_map(|r| r.check(value).err());
            if let Some(msg) = failed {
                problems.0.insert(field, msg);
            }
        }
        if !problems.is_empty() {
            return Verdict::Reject(problems);
        }
        Verdict::Accept(Message { name: trimmed(sub.name), email: trimmed(sub.email), body: trimmed(sub.message) })
    }
}

/// trim without a new allocation when there is nothing to trim
fn trimmed(s: String) -> String {
    match s.trim() {
        t if t.len() == s.len() => s,
        t => t.to_owned(),
    }
}

impl Default for Form {
    fn default() -> Self {
        Form::contact()
    }
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn good_submission_is_trimmed() {
        let v = Form::contact().check(Submission::new("  Rudo ", " rudo@example.com ", "\nHi\n"));
        let Verdict::Accept(m) = v else { panic!("{v:?}") };
        assert_eq!((m.name(), m.email(), m.body()), ("Rudo", "rudo@example.com", "Hi"));
    }

    #[test]
    fn every_problem_is_reported() {
        let Verdict::Reject(p) = Form::contact().check(Submission::new("", "nope", "  ")) else { panic!() };
        assert_eq!(p.len(), 3);
        assert_eq!(p.get(Field::Email), Some("Please check your email address."));
        assert_eq!(p.first().unwrap().0, Field::Name);
        assert!(p.to_string().starts_with("name: Please give your name."));
    }

    #[test]
    fn honeypot_means_bot() {
        let mut s = Submission::new("Bot", "b@b.co", "buy");
        s.honeypot = "http://spam".into();
        assert_eq!(Form::contact().check(s), Verdict::Bot);
    }

    #[test]
    fn limits_count_characters_not_bytes() {
        let name = "é".repeat(100);
        assert!(matches!(Form::contact().check(Submission::new(name, "a@b.co", "x")), Verdict::Accept(_)));
        let long = "é".repeat(101);
        assert!(matches!(Form::contact().check(Submission::new(long, "a@b.co", "x")), Verdict::Reject(_)));
    }

    #[test]
    fn control_characters_in_name_are_refused() {
        for bad in ["Eve\r\nBcc: x@y.z", "Eve\u{0}", "Eve\u{1b}[31m"] {
            assert!(matches!(Form::contact().check(Submission::new(bad, "a@b.co", "x")), Verdict::Reject(_)), "{bad:?}");
        }
    }

    #[test]
    fn custom_rules_extend_a_form() {
        let no_links = |v: &str| if v.contains("http") { Err("No links, please.") } else { Ok(()) };
        let form = Form::contact().rule(Field::Message, no_links);
        let Verdict::Reject(p) = form.check(Submission::new("A", "a@b.co", "see http://x")) else { panic!() };
        assert_eq!(p.get(Field::Message), Some("No links, please."));
    }
}
