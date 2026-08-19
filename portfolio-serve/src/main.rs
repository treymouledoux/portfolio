use axum::Router;
use tower_http::services::ServeDir;

#[tokio::main]
async fn main() {
    let app = Router::new().nest_service("/portfolio-static", ServeDir::new("assets"));

    let listener = tokio::net::TcpListener::bind("0.0.0.0:6050").await.unwrap();
    println!("Serving static files on http://localhost:6050");
    
    axum::serve(listener, app).await.unwrap();
}