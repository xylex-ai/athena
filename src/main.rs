use actix_cors::Cors;
use actix_web::body::{BoxBody, EitherBody};
use actix_web::dev::{Service, ServiceResponse};
use actix_web::get;
use actix_web::http::{header, StatusCode};
use actix_web::{web, App, HttpRequest, HttpResponse, HttpServer, Responder};
use dotenv::dotenv;
use reqwest::Method;
use serde_json::{json, Value};

use moka::future::Cache;
use reqwest::Client;
use std::env::var;
use std::io::Result;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;
use tracing_subscriber::EnvFilter;
use web::Data;

pub type SharedCache = Arc<Mutex<Cache<String, Value>>>;

const TARGET_BASE_URL: &str = "https://db.xylex.cloud";

#[get("/")]
async fn ping() -> impl Responder {
    println!("Received ping request");
    HttpResponse::Ok().json(json!({"message": "pong"}))
}

async fn proxy_request(req: HttpRequest, body: web::Bytes) -> impl Responder {
    println!("Proxy request received: {:?}", req.full_url().path());
    let client: Client = Client::new();
    let full_url: reqwest::Url = req.full_url();
    let full_url_path: &str = full_url.path();

    // Remove "/rest/v1" from the path if it exists
    let path: String = full_url_path.replacen("/rest/v1", "", 1);
    println!("Processed path: {}", path);

    let target_url: String = format!("{}{}", TARGET_BASE_URL, path);
    println!("Target URL: {}", target_url);

    // Convert actix_web::http::Method to reqwest::Method
    let reqwest_method = match req.method() {
        &actix_web::http::Method::GET => reqwest::Method::GET,
        &actix_web::http::Method::POST => reqwest::Method::POST,
        &actix_web::http::Method::PUT => reqwest::Method::PUT,
        &actix_web::http::Method::DELETE => reqwest::Method::DELETE,
        &actix_web::http::Method::HEAD => reqwest::Method::HEAD,
        &actix_web::http::Method::OPTIONS => reqwest::Method::OPTIONS,
        &actix_web::http::Method::CONNECT => reqwest::Method::CONNECT,
        &actix_web::http::Method::PATCH => reqwest::Method::PATCH,
        &actix_web::http::Method::TRACE => reqwest::Method::TRACE,
        _ => reqwest::Method::GET, // Default to GET if method is unknown
    };

    let mut client_req: reqwest::RequestBuilder = client.request(reqwest_method, &target_url);

    // Copy headers
    for (key, value) in req.headers().iter() {
        if key != header::HOST {
            client_req = client_req.header(
                reqwest::header::HeaderName::from_bytes(key.as_ref()).unwrap(),
                reqwest::header::HeaderValue::from_bytes(value.as_bytes()).unwrap(),
            );
        }
    }
    println!("Headers copied to client request");

    // Send request
    let client_response = client_req.body(body).send().await;
    println!("Request sent to target URL");

    match client_response {
        Ok(res) => {
            println!("Received response from target URL with status: {}", res.status());
            let mut client_resp = HttpResponse::build(
                actix_web::http::StatusCode::from_u16(res.status().as_u16()).unwrap(),
            );
            for (key, value) in res.headers().iter() {
                if key != &reqwest::header::CONTENT_ENCODING
                    && key != &reqwest::header::CONTENT_LENGTH
                    && key != &reqwest::header::TRANSFER_ENCODING
                    && key != &reqwest::header::CONNECTION
                {
                    if let (Ok(actix_key), Ok(actix_value)) = (
                        actix_web::http::header::HeaderName::try_from(key.as_str()),
                        actix_web::http::header::HeaderValue::from_str(
                            value.to_str().unwrap_or(""),
                        ),
                    ) {
                        client_resp.append_header((actix_key, actix_value));
                    }
                }
            }
            let body = res.bytes().await.unwrap_or_default();
            client_resp.body(body)
        }
        Err(e) => {
            println!("Error occurred while sending request: {:?}", e);
            HttpResponse::InternalServerError().finish()
        }
    }
}

#[actix_web::main]
async fn main() -> Result<()> {
    println!("Starting server...");
    init_tracing();
    dotenv().ok();
    let port: u16 = var("XLX_ATHENA_PORT")
        .unwrap_or("4052".to_string())
        .parse()
        .unwrap_or(4052);
    println!("Server will run on port: {}", port);

    let cache: SharedCache = Arc::new(Mutex::new(
        Cache::builder()
            .time_to_live(Duration::from_secs(60))
            .build(),
    ));
    println!("Cache initialized with TTL of 60 seconds");

    HttpServer::new(move || {
        let cors: Cors = Cors::default()
            .allow_any_origin()
            .allow_any_method()
            .allow_any_header();
        println!("CORS configured to allow any origin, method, and header");

        App::new()
            .wrap(cors)
            .wrap_fn(|req, srv| {
                let fut = srv.call(req);
                async move {
                    let mut res: ServiceResponse<EitherBody<BoxBody>> = fut.await?;
                    res.headers_mut()
                        .insert(header::SERVER, "XYLEX/0".parse().unwrap());
                    Ok(res)
                }
            })
            .app_data(Data::new(cache.clone()))
            .service(ping)
            .default_service(web::route().to(proxy_request))
    })
    .workers(16)
    .bind(("0.0.0.0", port))?
    .run()
    .await
}

fn init_tracing() {
    let filter: EnvFilter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));

    tracing_subscriber::fmt().with_env_filter(filter).init();
    println!("Tracing initialized with environment filter");
}
