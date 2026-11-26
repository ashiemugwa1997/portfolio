# contact-guard

Contact forms without the usual holes: field rules that report every problem at once, a honeypot instead of a captcha, and email composition where nothing the visitor typed can add headers.

```rust
use contact_guard::{Field, Form, PlainEmail, Submission, Verdict};

let form = Form::contact()                       // sensible limits for name, email, message
    .rule(Field::Message, |v: &str| {
        if v.contains("http") { Err("No links, please.") } else { Ok(()) }
    });

match form.check(Submission::new("Rudo", "rudo@example.com", "Hello!")) {
    Verdict::Accept(msg) => {
        let raw = PlainEmail::new("me@example.com", "me@example.com", &msg)
            .render("Wed, 26 Nov 2026 10:00:00 +0200");
        // hand `raw` to your SMTP client
    }
    Verdict::Reject(problems) => {
        for (field, message) in problems.iter() { /* show next to the field */ }
    }
    Verdict::Bot => { /* answer 202 as if accepted, keep nothing */ }
}
```

## Built-in rules

`Required`, `MaxChars` (characters, not bytes), `NoControl` (line breaks, NUL, escape codes, bidi overrides) and `EmailShape` (plain `local@domain.tld`, no display names or comments). Any `Fn(&str) -> Result<(), &'static str>` is a rule too.

## Email safety

- `header_safe` folds line breaks and control characters into spaces, so a name like `Eve\r\nBcc: victim@example.com` stays on one line.
- Non-ASCII subjects are RFC 2047 encoded.
- Body lines starting with `.` are dot-stuffed, so a lone `.` cannot end SMTP `DATA` early.
- Reply-To is the visitor; From is always your own address, so SPF/DMARC stay happy.

With the `lettre` feature, `msg.to_lettre(from, to, "Contact form message from")` gives you a ready-to-send `lettre::Message`.

With the `serde` feature, `Submission` deserializes from JSON, rejects unknown fields and accepts the honeypot as `website`.

## Licence

MIT or Apache-2.0, at your option.
