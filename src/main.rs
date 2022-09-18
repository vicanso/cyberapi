use axum::{
    extract::Path,
    http::StatusCode,
    response::Html,
    response::IntoResponse,
    routing::{get, get_service},
    Json, Router,
};
use pulldown_cmark::{html, Options, Parser};
use serde::Serialize;
use std::{env, fs, io, net::SocketAddr, path};
use tower_http::{services::ServeDir, trace::TraceLayer};

fn get_static_path() -> String {
    env::var("STATIC_PATH").unwrap()
}

#[tokio::main]
async fn main() {
    let static_path = get_static_path();

    // initialize tracing
    tracing_subscriber::fmt::init();

    // build our application with a route
    let app = Router::new()
        .route("/", get(root))
        .route("/docs/:name", get(root))
        .route("/api/markdowns/v1/:name", get(markdown))
        .fallback_service(get_service(ServeDir::new(static_path)).handle_error(handle_error))
        .layer(TraceLayer::new_for_http());

    // run our app with hyper
    // `axum::Server` is a re-export of `hyper::Server`
    let addr = SocketAddr::from(([127, 0, 0, 1], 3000));
    tracing::debug!("listening on {}", addr);
    axum::Server::bind(&addr)
        .serve(app.into_make_service())
        .await
        .unwrap();
}

async fn handle_error(_err: io::Error) -> impl IntoResponse {
    (StatusCode::INTERNAL_SERVER_ERROR, "Something went wrong...")
}

// basic handler that responds with a static string
async fn root() -> Html<String> {
    let static_path = get_static_path();
    let file = path::Path::new(static_path.as_str()).join("index.html");
    let result = fs::read_to_string(file).unwrap();
    Html(result)
}

#[derive(Serialize)]
struct MarkdownResult {
    html: String,
}

async fn markdown(Path(name): Path<String>) -> Json<MarkdownResult> {
    let static_path = get_static_path();
    let file = path::Path::new(static_path.as_str()).join(format!("docs/{}.md", name));
    let result = fs::read_to_string(file).unwrap();
    let mut options = Options::empty();
    options.insert(Options::ENABLE_STRIKETHROUGH);
    let parser = Parser::new_ext(result.as_str(), options);

    // Write to String buffer.
    let mut html_output = String::new();
    html::push_html(&mut html_output, parser);
    let resp = MarkdownResult { html: html_output };
    Json(resp)
}
