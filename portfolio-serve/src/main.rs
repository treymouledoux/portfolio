use axum::{Json, Router, http::StatusCode, routing::post};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use tokio::io::AsyncWriteExt;
use tokio::net::TcpListener;
use tower_http::{compression::CompressionLayer, services::ServeDir};

// What the browser sends.
#[derive(Deserialize, Debug)]
struct ContactForm {
    name: String,
    email: String,
    message: String,
}

// What writes to disk
#[derive(Serialize, Debug)]
struct StoredSubmission {
    received_at: String, // RFC 3339
    name: String,
    email: String,
    message: String,
}

#[derive(Serialize)]
struct ContactResponse {
    success: bool,
    reply_message: String,
}

fn validate(form: &ContactForm) -> Result<(), &'static str> {
    if form.name.trim().is_empty() {
        return Err("Please enter your name.");
    }
    if form.name.len() > 100 {
        return Err("Name is too long.");
    }
    let email = form.email.trim();
    if !email.contains('@') || !email.contains('.') || email.len() > 254 {
        return Err("Please enter a valid email address.");
    }
    let msg = form.message.trim();
    if msg.is_empty() {
        return Err("Please enter a message.");
    }
    if msg.len() > 5000 {
        return Err("Message is too long (5000 characters max).");
    }
    Ok(())
}

async fn save_submission(form: &ContactForm) -> std::io::Result<()> {
    let record = StoredSubmission {
        received_at: Utc::now().to_rfc3339(),
        name: form.name.trim().to_owned(),
        email: form.email.trim().to_owned(),
        message: form.message.trim().to_owned(),
    };

    let mut line = serde_json::to_string(&record).map_err(std::io::Error::other)?;
    line.push('\n');

    let mut file = tokio::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open("submissions.jsonl")
        .await?;
    file.write_all(line.as_bytes()).await?;
    Ok(())
}

async fn handle_contact_form_response(
    Json(payload): Json<ContactForm>,
) -> (StatusCode, Json<ContactResponse>) {
    // 1. Validate.
    if let Err(reason) = validate(&payload) {
        return (
            StatusCode::BAD_REQUEST,
            Json(ContactResponse {
                success: false,
                reply_message: reason.to_owned(),
            }),
        );
    }

    if let Err(e) = save_submission(&payload).await {
        eprintln!("failed to save submission: {e}");
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ContactResponse {
                success: false,
                reply_message: "Something went wrong on our end — please try again.".to_owned(),
            }),
        );
    }

    println!("New message from: {} ({})", payload.name, payload.email);

    (
        StatusCode::OK,
        Json(ContactResponse {
            success: true,
            reply_message: format!(
                "Thanks {}, we'll get back to you soon!",
                payload.name.trim()
            ),
        }),
    )
}

#[tokio::main]
async fn main() {
    let app = Router::new()
        .route("/api/contact", post(handle_contact_form_response))
        .fallback_service(ServeDir::new("assets").append_index_html_on_directories(true))
        .layer(CompressionLayer::new());

    let listener = TcpListener::bind("0.0.0.0:6050").await.unwrap();
    println!("Serving static files on http://localhost:6050");
    axum::serve(listener, app).await.unwrap();
}
