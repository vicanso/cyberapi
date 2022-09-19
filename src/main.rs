use axum::{routing::get, Router};
use std::net::SocketAddr;
use tower_http::trace::TraceLayer;

mod web;

#[tokio::main]
async fn main() {
    // let static_path = get_static_path();

    // initialize tracing
    tracing_subscriber::fmt::init();

    // build our application with a route
    let app = Router::new()
        .merge(web::new_router())
        // .route("/", get(root))
        .route("/ping", get(ping))
        .layer(TraceLayer::new_for_http());

    // run our app with hyper
    // `axum::Server` is a re-export of `hyper::Server`
    let addr = SocketAddr::from(([0, 0, 0, 0], 3000));
    tracing::debug!("listening on {}", addr);
    axum::Server::bind(&addr)
        .serve(app.into_make_service())
        .await
        .unwrap();
}

async fn ping() -> &'static str {
    "pong"
}
