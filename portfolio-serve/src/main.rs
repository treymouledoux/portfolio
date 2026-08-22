use axum::{Json, Router, routing::post};
use serde::{Deserialize, Serialize};
use tokio::net::TcpListener;
use tower_http::{compression::CompressionLayer, services::ServeDir};

#[derive(Deserialize, Debug)]
struct ContactForm {
    name: String,
    email: String,
    message: String,
}

#[derive(Serialize)]
struct ContactResponse {
    success: bool,
    reply_message: String,
}

async fn handle_contact_form_response(Json(payload): Json<ContactForm>) -> Json<ContactResponse> {
    //TODO: Implement email sending logic
    println!("New message from: {} ({})", payload.name, payload.email);
    println!("Message: {}", payload.message);

    Json(ContactResponse {
        success: true,
        reply_message: format!("Thanks {}, we'll get back to you soon!", payload.name),
    })
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
