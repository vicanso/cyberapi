use axum::{routing::get, Router};
use std::net::SocketAddr;

mod web;

#[tokio::main]
async fn main() {

    // build our application with a route
    let app = Router::new()
        .merge(web::new_router())
        // .route("/", get(root))
        .route("/ping", get(ping));

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
