use std::{
    env, io,
    path::{Path, PathBuf},
    time::Duration,
};

use lettre::{
    Address, AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor,
    message::{Mailbox, header::ContentType},
    transport::smtp::authentication::Credentials,
};
use serde::Deserialize;
use tokio::sync::Semaphore;

use crate::StoredSubmission;

const CREDENTIAL: &str = "portfolio-contact-smtp.json";
/// The dashboard's Settings view writes this list through the host agent, and
/// the site reads it for every delivery, so a saved change applies without a
/// restart of either service. Override the location with
/// `PORTFOLIO_CONTACT_RECIPIENTS`.
const RECIPIENTS: &str = "/etc/portfolio-contact/recipients.json";
const RECIPIENT_LIMIT: usize = 10;
const SEND_TIMEOUT: Duration = Duration::from_secs(8);

#[derive(Deserialize)]
struct Settings {
    host: String,
    port: u16,
    username: String,
    password: String,
    from: Address,
    /// The pre-dashboard configuration. It still delivers when no editable
    /// list exists, so an existing host keeps notifying without a migration.
    #[serde(default)]
    to: Option<Address>,
}

#[derive(Deserialize)]
struct RecipientFile {
    recipients: Vec<String>,
}

pub(super) struct ContactMailer {
    transport: AsyncSmtpTransport<Tokio1Executor>,
    sender: Mailbox,
    fallback: Vec<Mailbox>,
    path: PathBuf,
    sends: Semaphore,
}

impl ContactMailer {
    pub(super) fn load() -> io::Result<Option<Self>> {
        let Some(path) = env::var_os("CREDENTIALS_DIRECTORY")
            .map(|directory| PathBuf::from(directory).join(CREDENTIAL))
        else {
            return Ok(None);
        };
        let contents = match std::fs::read(path) {
            Ok(contents) => contents,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error),
        };
        let settings: Settings = serde_json::from_slice(&contents).map_err(|_| {
            io::Error::new(io::ErrorKind::InvalidData, "invalid SMTP configuration")
        })?;
        if settings.host.trim().is_empty()
            || settings.port == 0
            || settings.username.trim().is_empty()
            || settings.password.trim().is_empty()
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "incomplete SMTP configuration",
            ));
        }
        let transport = AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(&settings.host)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "invalid SMTP host"))?
            .port(settings.port)
            .credentials(Credentials::new(settings.username, settings.password))
            .build();
        let recipients = env::var_os("PORTFOLIO_CONTACT_RECIPIENTS")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(RECIPIENTS));
        Ok(Some(Self {
            transport,
            sender: Mailbox::new(None, settings.from),
            fallback: settings
                .to
                .into_iter()
                .map(|address| Mailbox::new(None, address))
                .collect(),
            path: recipients,
            sends: Semaphore::new(8),
        }))
    }

    /// The saved list wins; a missing, empty or unreadable file leaves the
    /// credential's own address in place rather than silently dropping mail.
    /// The journal names the path, because a sandbox that hides the file looks
    /// like an ordinary single delivery and hides the mistake.
    fn recipients(&self) -> Vec<Mailbox> {
        let reason = match read_recipients(&self.path) {
            Some(saved) if !saved.is_empty() => return saved,
            Some(_) => "empty",
            None => "unreadable",
        };
        eprintln!(
            "Contact recipients at {} are {reason}; using the mail credential's address",
            self.path.display()
        );
        self.fallback.clone()
    }

    fn message(
        &self,
        submission: &StoredSubmission,
        recipients: &[Mailbox],
    ) -> Result<Message, lettre::error::Error> {
        let mut email = Message::builder().from(self.sender.clone());
        for recipient in recipients {
            email = email.to(recipient.clone());
        }
        let mut email = email
            .subject("New portfolio contact message")
            .header(ContentType::TEXT_PLAIN);
        if let Ok(address) = submission.email.parse::<Address>() {
            email = email.reply_to(Mailbox::new(None, address));
        }
        email.body(format!(
            "Name: {}\nEmail: {}\nReceived: {}\n\n{}",
            submission.name, submission.email, submission.received_at, submission.message,
        ))
    }

    pub(super) async fn notify(&self, submission: &StoredSubmission) {
        // Read per delivery, not at startup: the dashboard's Settings view is
        // the owner's only editor and must not need a service restart.
        let recipients = self.recipients();
        if recipients.is_empty() {
            eprintln!("Contact email has no recipient configured; submission saved");
            return;
        }
        let result = tokio::time::timeout(SEND_TIMEOUT, async {
            let _permit = self.sends.acquire().await.map_err(|_| ())?;
            let email = self.message(submission, &recipients).map_err(|_| ())?;
            self.transport.send(email).await.map_err(|_| ())
        })
        .await;
        if !matches!(result, Ok(Ok(_))) {
            eprintln!("Contact email delivery was not confirmed; submission saved");
        }
    }
}

fn read_recipients(path: &Path) -> Option<Vec<Mailbox>> {
    let contents = std::fs::read(path).ok()?;
    let file: RecipientFile = serde_json::from_slice(&contents).ok()?;
    Some(parse_recipients(&file.recipients))
}

/// Anything the dashboard would not save is skipped rather than turning the
/// whole list off, and a repeated address is only mailed once.
fn parse_recipients(values: &[String]) -> Vec<Mailbox> {
    let mut seen: Vec<String> = Vec::new();
    let mut recipients = Vec::new();
    for value in values.iter().take(RECIPIENT_LIMIT) {
        let Ok(address) = value.trim().parse::<Address>() else {
            continue;
        };
        let key = address.to_string().to_ascii_lowercase();
        if seen.contains(&key) {
            continue;
        }
        seen.push(key);
        recipients.push(Mailbox::new(None, address));
    }
    recipients
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mailer(path: PathBuf, fallback: &[&str]) -> ContactMailer {
        ContactMailer {
            transport: AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous("localhost").build(),
            sender: "sender@example.test".parse().unwrap(),
            fallback: fallback
                .iter()
                .map(|address| Mailbox::new(None, address.parse().unwrap()))
                .collect(),
            path,
            sends: Semaphore::new(1),
        }
    }

    fn submission() -> StoredSubmission {
        StoredSubmission {
            received_at: "2026-11-16T15:00:00+00:00".into(),
            name: "Visitor".into(),
            email: "visitor@example.test".into(),
            message: "Hello there.".into(),
        }
    }

    #[test]
    fn sends_to_every_saved_address_and_sets_reply_to() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("recipients.json");
        std::fs::write(
            &path,
            br#"{"recipients":["one@example.test","two@example.test","junk","ONE@example.test"]}"#,
        )
        .unwrap();
        let mailer = mailer(path, &["fallback@example.test"]);
        let recipients = mailer.recipients();
        // The duplicate and the unparseable entry are dropped, not delivered.
        assert_eq!(recipients.len(), 2);
        let email = mailer.message(&submission(), &recipients).unwrap();
        assert_eq!(
            email.envelope().to().to_vec(),
            vec![
                "one@example.test".parse::<Address>().unwrap(),
                "two@example.test".parse::<Address>().unwrap(),
            ]
        );
        let formatted = String::from_utf8(email.formatted()).unwrap();
        assert!(formatted.contains("one@example.test"));
        assert!(formatted.contains("two@example.test"));
        assert!(formatted.contains("Reply-To: visitor@example.test\r\n"));
        assert!(formatted.contains("Hello there."));
    }

    #[test]
    fn a_missing_or_empty_file_keeps_the_credential_recipient() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("recipients.json");
        let mailer = mailer(path.clone(), &["fallback@example.test"]);
        assert_eq!(mailer.recipients().len(), 1);
        std::fs::write(&path, b"{}").unwrap();
        assert_eq!(mailer.recipients().len(), 1);
        std::fs::write(&path, br#"{"recipients":[]}"#).unwrap();
        assert_eq!(mailer.recipients().len(), 1);
        std::fs::write(&path, br#"{"recipients":["not an address"]}"#).unwrap();
        assert_eq!(mailer.recipients().len(), 1);
    }
}
