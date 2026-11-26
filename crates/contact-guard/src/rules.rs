/// A check on one field's trimmed value. `Err` carries the message shown to the visitor.
pub trait FieldRule: Send + Sync {
    /// Check `value`.
    fn check(&self, value: &str) -> Result<(), &'static str>;
}

impl<F> FieldRule for F
where
    F: Fn(&str) -> Result<(), &'static str> + Send + Sync,
{
    fn check(&self, value: &str) -> Result<(), &'static str> {
        self(value)
    }
}

/// Must not be empty.
#[derive(Debug, Clone, Copy)]
pub struct Required(pub &'static str);

impl FieldRule for Required {
    fn check(&self, v: &str) -> Result<(), &'static str> {
        if v.is_empty() {
            Err(self.0)
        } else {
            Ok(())
        }
    }
}

/// At most this many characters (not bytes).
#[derive(Debug, Clone, Copy)]
pub struct MaxChars(pub usize, pub &'static str);

impl FieldRule for MaxChars {
    fn check(&self, v: &str) -> Result<(), &'static str> {
        // bytes >= chars, so short strings skip the count
        if v.len() <= self.0 || v.chars().nth(self.0).is_none() {
            Ok(())
        } else {
            Err(self.1)
        }
    }
}

/// No control characters (line breaks, NUL, escape codes, bidi overrides).
#[derive(Debug, Clone, Copy)]
pub struct NoControl(pub &'static str);

impl FieldRule for NoControl {
    fn check(&self, v: &str) -> Result<(), &'static str> {
        let bad = v.chars().any(|c| c.is_control() || matches!(c, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}'));
        if bad {
            Err(self.0)
        } else {
            Ok(())
        }
    }
}

/// A plain `local@domain.tld` address: no display names, comments, quotes or spaces,
/// a dotted domain whose labels are letters, digits and inner hyphens.
#[derive(Debug, Clone, Copy)]
pub struct EmailShape(pub &'static str);

impl FieldRule for EmailShape {
    fn check(&self, v: &str) -> Result<(), &'static str> {
        if is_plain_address(v) {
            Ok(())
        } else {
            Err(self.0)
        }
    }
}

pub(crate) fn is_plain_address(v: &str) -> bool {
    let Some((local, domain)) = v.rsplit_once('@') else { return false };
    let local_ok = !local.is_empty()
        && local.len() <= 64
        && !local.starts_with('.')
        && !local.ends_with('.')
        && !local.contains("..")
        && local.chars().all(|c| c.is_ascii_alphanumeric() || "!#$%&'*+-/=?^_`{|}~.".contains(c));
    let labels: Vec<&str> = domain.split('.').collect();
    let domain_ok = domain.len() <= 253
        && labels.len() >= 2
        && labels.iter().all(|l| {
            !l.is_empty()
                && l.len() <= 63
                && !l.starts_with('-')
                && !l.ends_with('-')
                && l.chars().all(|c| c.is_alphanumeric() || c == '-')
        })
        && labels.last().is_some_and(|tld| tld.chars().all(char::is_alphabetic) && tld.chars().count() >= 2);
    local_ok && domain_ok
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn addresses() {
        for good in ["a@b.co", "first.last+tag@sub.example.com", "x@zesa.co.zw", "o'neil@example.ie", "a@bücher.de"] {
            assert!(is_plain_address(good), "{good}");
        }
        for bad in [
            "", "a", "a@", "@b.co", "a@b", "a@b.c", "a b@c.co", "A <a@b.co>", "a@b.co\r\nBcc: x@y.z", "a..b@c.co",
            ".a@b.co", "a@-b.co", "a@b-.co", "a@b..co", "a@b.co.", "\"a\"@b.co", "a@b.c0m", "a@[127.0.0.1]",
        ] {
            assert!(!is_plain_address(bad), "{bad:?}");
        }
    }

    #[test]
    fn max_chars_on_the_boundary() {
        let r = MaxChars(3, "too long");
        assert!(r.check("abc").is_ok());
        assert!(r.check("ééé").is_ok(), "6 bytes, 3 characters");
        assert!(r.check("abcd").is_err());
    }

    #[test]
    fn bidi_overrides_are_control() {
        assert!(NoControl("x").check("abc\u{202e}cba").is_err());
    }
}
