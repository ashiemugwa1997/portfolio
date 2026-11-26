# Changelog

## 0.2.0

- Feature `lettre`: `Message::to_lettre(from, to, subject_prefix)` builds a ready-to-send `lettre::Message` (reply-to is the visitor, every header folded to one line).
- Property tests: tens of thousands of hostile inputs (line breaks, NUL, escape codes, bidi overrides, lone dots) against the form and the renderer.

## 0.1.0

- First release: field rules, honeypot, header-injection-safe `PlainEmail`.
