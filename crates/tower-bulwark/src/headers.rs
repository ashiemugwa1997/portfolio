use http::header::{HeaderMap, HeaderName, HeaderValue};

/// Headers added to every response unless the handler set them already.
#[derive(Debug, Clone, Default)]
pub struct HeaderPolicy {
    headers: Vec<(HeaderName, HeaderValue)>,
}

impl HeaderPolicy {
    /// A strict set for sites that do not need to be framed or embedded elsewhere.
    pub fn strict(csp: &str) -> Self {
        HeaderPolicy::default()
            .with("content-security-policy", csp)
            .with("x-content-type-options", "nosniff")
            .with("x-frame-options", "DENY")
            .with("referrer-policy", "strict-origin-when-cross-origin")
            .with("permissions-policy", "camera=(), microphone=(), geolocation=(), payment=(), usb=()")
            .with("cross-origin-opener-policy", "same-origin")
            .with("cross-origin-resource-policy", "same-origin")
    }

    /// Add or replace a header.
    ///
    /// # Panics
    /// If `name` or `value` is not a valid header (a programming error, caught at start-up).
    #[must_use]
    pub fn with(self, name: &str, value: &str) -> Self {
        let name = HeaderName::from_bytes(name.as_bytes()).expect("valid header name");
        let value = HeaderValue::from_str(value).expect("valid header value");
        self.with_value(name, value)
    }

    /// Add or replace a header.
    #[must_use]
    pub fn with_value(mut self, name: HeaderName, value: HeaderValue) -> Self {
        match self.headers.iter_mut().find(|(n, _)| *n == name) {
            Some(slot) => slot.1 = value,
            None => self.headers.push((name, value)),
        }
        self
    }

    /// The value configured for `name`.
    pub fn get(&self, name: &str) -> Option<&HeaderValue> {
        self.headers.iter().find(|(n, _)| n.as_str().eq_ignore_ascii_case(name)).map(|(_, v)| v)
    }

    pub(crate) fn apply(&self, target: &mut HeaderMap) {
        for (name, value) in &self.headers {
            if !target.contains_key(name) {
                target.insert(name.clone(), value.clone());
            }
        }
    }
}
