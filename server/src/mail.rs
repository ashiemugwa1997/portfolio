use std::future::Future;
use std::net::{IpAddr, SocketAddr};
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use contact_guard::Message;
use lettre::message::Mailbox;
use lettre::transport::smtp::authentication::Credentials;
use lettre::transport::smtp::client::{Tls, TlsParameters};
use lettre::{AsyncSmtpTransport, AsyncTransport, Tokio1Executor};
use tokio::sync::Semaphore;

pub type MailFuture<'a> = Pin<Box<dyn Future<Output = Result<(), String>> + Send + 'a>>;

pub trait Mailer: Send + Sync + 'static {
    fn send<'a>(&'a self, m: &'a Message) -> MailFuture<'a>;
}

// ── SMTP ────────────────────────────────────────────────────────────────────

/// Sends through an SMTP relay with STARTTLS (Gmail or iCloud with an app password).
///
/// Addresses are resolved per send and tried IPv4 first: on networks where IPv6 is listed
/// but not routed, a client that takes the first answer waits out its whole timeout.
/// The certificate is still checked against the relay's name, not the address.
pub struct Smtp {
    host: String,
    port: u16,
    creds: Credentials,
    tls: TlsParameters,
    from: Mailbox,
    to: Mailbox,
}

impl Smtp {
    pub fn new(host: &str, port: u16, user: &str, pass: &str, from: &str, to: &str) -> Result<Self, String> {
        let tls = TlsParameters::new(host.to_owned()).map_err(|e| e.to_string())?;
        let from = format!("Portfolio <{from}>").parse::<Mailbox>().map_err(|e| e.to_string())?;
        let to = to.parse::<Mailbox>().map_err(|e| e.to_string())?;
        Ok(Smtp { host: host.to_owned(), port, creds: Credentials::new(user.to_owned(), pass.to_owned()), tls, from, to })
    }

    fn transport(&self, ip: IpAddr) -> AsyncSmtpTransport<Tokio1Executor> {
        AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(ip.to_string())
            .port(self.port)
            .tls(Tls::Required(self.tls.clone()))
            .credentials(self.creds.clone())
            .timeout(Some(Duration::from_secs(15)))
            .build()
    }
}

/// IPv4 first, then IPv6, each family in resolver order
fn ipv4_first(addrs: impl IntoIterator<Item = SocketAddr>) -> Vec<IpAddr> {
    let (mut v4, v6): (Vec<IpAddr>, Vec<IpAddr>) = addrs.into_iter().map(|a| a.ip()).partition(IpAddr::is_ipv4);
    v4.extend(v6);
    v4.dedup();
    v4
}

impl Mailer for Smtp {
    fn send<'a>(&'a self, m: &'a Message) -> MailFuture<'a> {
        Box::pin(async move {
            let email = m.to_lettre(self.from.clone(), self.to.clone(), "Portfolio message from").map_err(|e| e.to_string())?;

            let addrs = tokio::net::lookup_host((self.host.as_str(), self.port)).await.map_err(|e| format!("{}: {e}", self.host))?;
            let mut last = format!("{}: no addresses", self.host);
            for ip in ipv4_first(addrs) {
                match self.transport(ip).send(email.clone()).await {
                    Ok(_) => return Ok(()),
                    Err(e) => last = format!("{ip}: {e}"),
                }
            }
            Err(last)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ipv4_addresses_come_first() {
        let addrs: Vec<SocketAddr> = ["[2a00:1450::6d]:587", "64.233.167.108:587", "[2a00:1450::6c]:587", "64.233.167.109:587"]
            .iter()
            .map(|a| a.parse().unwrap())
            .collect();
        let order: Vec<String> = ipv4_first(addrs).iter().map(ToString::to_string).collect();
        assert_eq!(order, ["64.233.167.108", "64.233.167.109", "2a00:1450::6d", "2a00:1450::6c"]);
    }
}
