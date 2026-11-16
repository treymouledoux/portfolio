use std::{env, io, path::PathBuf, time::Duration};

use lettre::{
    Address, AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor,
    message::{Mailbox, header::ContentType},
    transport::smtp::authentication::Credentials,
};
use serde::Deserialize;
use tokio::sync::Semaphore;

use crate::StoredSubmission;

const CREDENTIAL: &str = "portfolio-contact-smtp.json";
const SEND_TIMEOUT: Duration = Duration::from_secs(8);

#[derive(Deserialize)]
struct Settings {
    host: String,
    port: u16,
    username: String,
    password: String,
    from: Address,
    to: Address,
}

pub(super) struct ContactMailer {
    transport: AsyncSmtpTransport<Tokio1Executor>,
    sender: Mailbox,
    recipient: Mailbox,
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
        Ok(Some(Self {
            transport,
            sender: Mailbox::new(None, settings.from),
            recipient: Mailbox::new(None, settings.to),
            sends: Semaphore::new(8),
        }))
    }

    fn message(&self, submission: &StoredSubmission) -> Result<Message, lettre::error::Error> {
        let mut email = Message::builder()
            .from(self.sender.clone())
            .to(self.recipient.clone())
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
        let result = tokio::time::timeout(SEND_TIMEOUT, async {
            let _permit = self.sends.acquire().await.map_err(|_| ())?;
            let email = self.message(submission).map_err(|_| ())?;
            self.transport.send(email).await.map_err(|_| ())
        })
        .await;
        if !matches!(result, Ok(Ok(_))) {
            eprintln!("Contact email delivery was not confirmed; submission saved");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sends_only_to_the_configured_address_and_sets_reply_to() {
        let mailer = ContactMailer {
            transport: AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous("localhost").build(),
            sender: "sender@example.test".parse().unwrap(),
            recipient: "owner@example.test".parse().unwrap(),
            sends: Semaphore::new(1),
        };
        let submission = StoredSubmission {
            received_at: "2026-11-16T15:00:00+00:00".into(),
            name: "Visitor".into(),
            email: "visitor@example.test".into(),
            message: "Hello there.".into(),
        };
        let email = mailer.message(&submission).unwrap();
        assert_eq!(
            email.envelope().to(),
            &["owner@example.test".parse::<Address>().unwrap()]
        );
        let formatted = String::from_utf8(email.formatted()).unwrap();
        assert!(formatted.contains("Reply-To: visitor@example.test\r\n"));
        assert!(formatted.contains("Hello there."));
    }
}
