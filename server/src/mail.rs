use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use contact_guard::{header_safe, Message};
use lettre::message::{header::ContentType, Mailbox};
use lettre::transport::smtp::authentication::Credentials;
use lettre::{AsyncSmtpTransport, AsyncTransport, Tokio1Executor};
use tokio::sync::Semaphore;

pub type MailFuture<'a> = Pin<Box<dyn Future<Output = Result<(), String>> + Send + 'a>>;

pub trait Mailer: Send + Sync + 'static {
    fn send<'a>(&'a self, m: &'a Message) -> MailFuture<'a>;
}

// ── SMTP ────────────────────────────────────────────────────────────────────

/// Sends through an SMTP relay with STARTTLS (Gmail with an app password by default).
pub struct Smtp {
    transport: AsyncSmtpTransport<Tokio1Executor>,
    from: Mailbox,
    to: Mailbox,
}

impl Smtp {
    pub fn new(host: &str, port: u16, user: &str, pass: &str, to: &str) -> Result<Self, String> {
        let transport = AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(host)
            .map_err(|e| e.to_string())?
            .port(port)
            .credentials(Credentials::new(user.to_owned(), pass.to_owned()))
            .timeout(Some(Duration::from_secs(20)))
            .build();
        let from = format!("Portfolio <{user}>").parse::<Mailbox>().map_err(|e| e.to_string())?;
        let to = to.parse::<Mailbox>().map_err(|e| e.to_string())?;
        Ok(Smtp { transport, from, to })
    }
}

impl Mailer for Smtp {
    fn send<'a>(&'a self, m: &'a Message) -> MailFuture<'a> {
        Box::pin(async move {
            let reply_to: Mailbox = m.email().parse().map_err(|e: lettre::address::AddressError| e.to_string())?;
            let email = lettre::Message::builder()
                .from(self.from.clone())
                .to(self.to.clone())
                .reply_to(reply_to)
                .subject(format!("Portfolio message from {}", header_safe(m.name())))
                .header(ContentType::TEXT_PLAIN)
                .body(format!("{} <{}> wrote:\n\n{}", header_safe(m.name()), m.email(), m.body()))
                .map_err(|e| e.to_string())?;
            self.transport.send(email).await.map(|_| ()).map_err(|e| e.to_string())
        })
    }
}

// ── Notifier ────────────────────────────────────────────────────────────────

/// Sends in the background, a couple at a time; extra messages skip email (they are
/// already in the database) instead of piling up tasks.
#[derive(Clone)]
pub struct Notifier {
    mailer: Option<Arc<dyn Mailer>>,
    slots: Arc<Semaphore>,
}

impl Notifier {
    pub fn new(mailer: Option<Arc<dyn Mailer>>) -> Self {
        Notifier { mailer, slots: Arc::new(Semaphore::new(2)) }
    }

    pub fn off() -> Self {
        Notifier::new(None)
    }

    pub fn notify(&self, m: Message) {
        let Some(mailer) = self.mailer.clone() else { return };
        let Ok(permit) = self.slots.clone().try_acquire_owned() else {
            tracing::warn!("email skipped: two sends already in flight (message is saved)");
            return;
        };
        tokio::spawn(async move {
            if let Err(e) = mailer.send(&m).await {
                tracing::warn!("email: {e}");
            }
            drop(permit);
        });
    }
}
