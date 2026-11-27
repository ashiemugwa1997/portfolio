use std::path::PathBuf;

use contact_guard::{Form, Submission, Verdict};
use serde::Deserialize;

use crate::store::Store;

pub struct Config {
    pub addr: String,
    pub static_dir: PathBuf,
    pub dsn: Option<String>,
    pub admin_dsn: Option<String>,
    pub trust_proxy: bool,
    pub https: bool,
    pub max_connections: usize,
    pub rate_limit: usize,
    /// secret mixed into sender hashes, so stored hashes cannot be matched to addresses
    pub ip_salt: Option<String>,
    smtp_user: Option<String>,
    smtp_pass: Option<String>,
    smtp_host: String,
    smtp_port: u16,
    mail_to: Option<String>,
    mail_from: Option<String>,
}

pub struct SmtpSettings {
    pub host: String,
    pub port: u16,
    pub user: String,
    pub pass: String,
    pub to: String,
    /// the sender address; iCloud signs in with the Apple ID but can send from a custom domain
    pub from: String,
}

impl Config {
    /// settings from the environment, after filling gaps from a KEY=value file
    pub fn load(env_file: &str) -> Self {
        load_env_file(env_file);
        let var = |k: &str| secret(k);
        let flag = |k: &str| var(k).is_some_and(|v| matches!(v.to_ascii_lowercase().as_str(), "1" | "true" | "yes"));
        Config {
            addr: var("ADDR").unwrap_or_else(|| "127.0.0.1:8080".into()),
            static_dir: var("STATIC_DIR").unwrap_or_else(|| "../web/dist".into()).into(),
            ip_salt: var("IP_HASH_SALT"),
            dsn: var("MYSQL_DSN"),
            admin_dsn: var("MYSQL_ADMIN_DSN"),
            trust_proxy: flag("TRUST_PROXY"),
            https: flag("HTTPS"),
            max_connections: var("MAX_CONNECTIONS").and_then(|v| v.parse().ok()).unwrap_or(512),
            rate_limit: var("RATE_LIMIT_PER_MIN").and_then(|v| v.parse().ok()).unwrap_or(120),
            smtp_user: var("SMTP_USER"),
            smtp_pass: var("SMTP_PASS").map(|p| p.replace(' ', "")),
            smtp_host: var("SMTP_HOST").unwrap_or_else(|| "smtp.gmail.com".into()),
            smtp_port: var("SMTP_PORT").and_then(|v| v.parse().ok()).unwrap_or(587),
            mail_to: var("MAIL_TO"),
            mail_from: var("MAIL_FROM"),
        }
    }

    pub fn smtp(&self) -> Option<SmtpSettings> {
        let user = self.smtp_user.clone()?;
        let pass = self.smtp_pass.clone()?;
        Some(SmtpSettings {
            host: self.smtp_host.clone(),
            port: self.smtp_port,
            to: self.mail_to.clone().unwrap_or_else(|| user.clone()),
            from: self.mail_from.clone().unwrap_or_else(|| user.clone()),
            user,
            pass,
        })
    }
}

/// A setting from `KEY`, or from the file named by `KEY_FILE` (Docker and systemd credentials),
/// so secrets need not sit in the environment or in .env.
fn secret(key: &str) -> Option<String> {
    let direct = std::env::var(key).ok().filter(|v| !v.trim().is_empty());
    direct.or_else(|| {
        let path = std::env::var(format!("{key}_FILE")).ok().filter(|v| !v.trim().is_empty())?;
        match std::fs::read_to_string(&path) {
            Ok(v) => Some(v.trim().to_owned()).filter(|v| !v.is_empty()),
            Err(e) => {
                tracing::warn!("{key}_FILE {path}: {e}");
                None
            }
        }
    })
}

/// KEY=value lines; variables already in the environment win
fn load_env_file(path: &str) {
    let Ok(text) = std::fs::read_to_string(path) else { return };
    for line in text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')) {
        let Some((k, v)) = line.split_once('=') else { continue };
        let (k, v) = (k.trim(), v.trim().trim_matches(|c| c == '"' || c == '\''));
        if std::env::var_os(k).is_none() {
            std::env::set_var(k, v);
        }
    }
}

#[derive(Deserialize)]
struct OldMessage {
    name: String,
    email: String,
    message: String,
    #[serde(default, with = "time::serde::rfc3339::option")]
    received: Option<time::OffsetDateTime>,
}

/// messages saved to a file before the database existed: move them in once
pub async fn import_old_messages<S: Store>(store: &S, path: &str) {
    let Ok(text) = tokio::fs::read_to_string(path).await else { return };
    let form = Form::contact();
    let mut moved = 0;
    for line in text.lines() {
        let Ok(old) = serde_json::from_str::<OldMessage>(line) else { continue };
        let Verdict::Accept(m) = form.check(Submission::new(old.name, old.email, old.message)) else { continue };
        if let Err(e) = store.save_message(&m, old.received.unwrap_or_else(time::OffsetDateTime::now_utc), None).await {
            tracing::warn!("import stopped: {e}");
            return;
        }
        moved += 1;
    }
    if tokio::fs::rename(path, format!("{path}.imported")).await.is_ok() {
        tracing::info!("import: {moved} old messages moved into MySQL");
    }
}

#[cfg(test)]
mod tests {
    use super::{secret, Config};

    fn with_mail(from: Option<&str>) -> Config {
        Config {
            addr: String::new(),
            static_dir: ".".into(),
            dsn: None,
            admin_dsn: None,
            trust_proxy: false,
            https: false,
            max_connections: 1,
            rate_limit: 1,
            ip_salt: None,
            smtp_user: Some("me@icloud.com".into()),
            smtp_pass: Some("x".into()),
            smtp_host: "smtp.mail.me.com".into(),
            smtp_port: 587,
            mail_to: None,
            mail_from: from.map(Into::into),
        }
    }

    #[test]
    fn mail_can_come_from_another_address_than_the_login() {
        let s = with_mail(Some("hello@samugwambi.site")).smtp().unwrap();
        assert_eq!((s.user.as_str(), s.from.as_str(), s.to.as_str()), ("me@icloud.com", "hello@samugwambi.site", "me@icloud.com"));
        assert_eq!(with_mail(None).smtp().unwrap().from, "me@icloud.com", "the login by default");
    }

    #[test]
    fn secrets_can_come_from_files() {
        let path = std::env::temp_dir().join(format!("pf-secret-{}", std::process::id()));
        std::fs::write(&path, "  from-a-file \n").unwrap();
        std::env::remove_var("PF_TEST_SECRET");
        std::env::set_var("PF_TEST_SECRET_FILE", &path);
        assert_eq!(secret("PF_TEST_SECRET").as_deref(), Some("from-a-file"));
        std::env::set_var("PF_TEST_SECRET", "direct");
        assert_eq!(secret("PF_TEST_SECRET").as_deref(), Some("direct"), "the variable itself wins");
        std::env::remove_var("PF_TEST_SECRET");
        std::env::set_var("PF_TEST_SECRET_FILE", "/no/such/file");
        assert_eq!(secret("PF_TEST_SECRET"), None);
        let _ = std::fs::remove_file(path);
    }
}
