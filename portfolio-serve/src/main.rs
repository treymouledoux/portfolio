use axum::Router;
use tower_http::services::ServeDir;

#[tokio::main]
async fn main() {
    let app = Router::new().fallback_service(
        ServeDir::new("assets").append_index_html_on_directories(true),
    );

    let listener = tokio::net::TcpListener::bind("0.0.0.0:6050").await.unwrap();
    println!("Serving static files on http://localhost:6050");
    axum::serve(listener, app).await.unwrap();
}
