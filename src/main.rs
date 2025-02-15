use actix_cors::Cors;
use actix_web::body::{BoxBody, EitherBody};
use actix_web::dev::{Service, ServiceResponse};
use actix_web::get;
use actix_web::http::{header, StatusCode};
use actix_web::{web, App, HttpRequest, HttpResponse, HttpServer, Responder};
use dotenv::dotenv;
use moka::future::Cache;
use reqwest::{Client, Method};
use serde_json::{json, Value};
use std::env::var;
use std::io::Result;
use std::sync::Arc;
use std::time::Duration;
use tracing::info;
use tracing_subscriber::EnvFilter;
use web::Data;

pub struct AppState {
    cache: Arc<Cache<String, Value>>, // Removed Mutex for async-safe cache
    client: Client,
}

const TARGET_BASE_URL: &str = "https://db.xylex.cloud";

#[get("/")]
async fn ping() -> impl Responder {
    info!("Received ping request");
    HttpResponse::Ok().json(json!({"message": "pong"}))
}

async fn proxy_request(
    req: HttpRequest,
    body: web::Bytes,
    app_state: Data<AppState>,
) -> impl Responder {
    let client = &app_state.client;
    let cache = &app_state.cache;
    let full_url = req.full_url();
    let full_url_path = full_url.path();
    let path = full_url_path.replacen("/rest/v1", "", 1);
    info!("path: {:#?}", path);

    let target_url: String = format!("{}{}", TARGET_BASE_URL, path);
    info!("target_url {:#?}", target_url);

    let jwt_token = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .map(|value| value.to_string())
        .unwrap_or_default();

    let cache_control_header: Option<header::HeaderValue> =
        req.headers().get(header::CACHE_CONTROL).cloned();

    let cachekey = format!("{}-{}-{}", req.method(), full_url, jwt_token)
        .replace('*', "_xXx_")
        .replace(' ', "_")
        .replace(':', "-")
        .replace('/', "_");

    if cache_control_header
        .as_ref()
        .map_or(true, |h| h != "no-cache")
    {
        if let Some(cached_response) = cache.get(&cachekey).await {
            return HttpResponse::Ok().json(cached_response);
        }
    }

    let reqwest_method = match *req.method() {
        actix_web::http::Method::GET => Method::GET,
        actix_web::http::Method::POST => Method::POST,
        actix_web::http::Method::PUT => Method::PUT,
        actix_web::http::Method::DELETE => Method::DELETE,
        actix_web::http::Method::PATCH => Method::PATCH,
        _ => Method::GET,
    };
    let mut client_req = client.request(reqwest_method, &target_url);
    for (key, value) in req.headers().iter() {
        if key != header::HOST {
            let reqwest_key = reqwest::header::HeaderName::from_bytes(key.as_ref()).unwrap();
            let reqwest_value = reqwest::header::HeaderValue::from_bytes(value.as_bytes()).unwrap();
            client_req = client_req.header(reqwest_key, reqwest_value);
        }
    }
    match client_req.body(body).send().await {
        Ok(res) => {
            let status_code = StatusCode::from_u16(res.status().as_u16()).unwrap();
            let headers = res.headers().clone();
            let body_bytes = res.bytes().await.unwrap_or_default();
            let json_body: Value = serde_json::from_slice(&body_bytes).unwrap_or(Value::Null);
            cache.insert(cachekey, json_body.clone()).await;
            let mut response = HttpResponse::build(status_code);

            for (key, value) in headers.iter() {
                if ![
                    reqwest::header::CONTENT_ENCODING,
                    reqwest::header::CONTENT_LENGTH,
                    reqwest::header::TRANSFER_ENCODING,
                    reqwest::header::CONNECTION,
                ]
                .contains(key)
                {
                    let actix_key =
                        actix_web::http::header::HeaderName::from_bytes(key.as_str().as_bytes())
                            .unwrap();
                    let actix_value =
                        actix_web::http::header::HeaderValue::from_bytes(value.as_bytes()).unwrap();
                    response.append_header((actix_key, actix_value));
                }
            }
            response.body(body_bytes)
        }
        Err(_) => HttpResponse::InternalServerError().finish(),
    }
}

#[actix_web::main]
async fn main() -> Result<()> {
    dotenv().ok();
    init_tracing();
    let port: u16 = var("XLX_ATHENA_PORT")
        .unwrap_or_else(|_| "4052".to_string())
        .parse()
        .unwrap_or(4052);
    let cache = Arc::new(
        Cache::builder()
            .time_to_live(Duration::from_secs(60))
            .build(),
    );
    let client = Client::builder()
        .pool_idle_timeout(Duration::from_secs(90))
        .build()
        .unwrap();
    let app_state = Data::new(AppState { cache, client });

    HttpServer::new(move || {
        let cors = Cors::default()
            .allow_any_origin()
            .allow_any_method()
            .allow_any_header();
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
            .app_data(app_state.clone())
            .service(ping)
            .default_service(web::route().to(proxy_request))
    })
    .workers(num_cpus::get() * 2)
    .keep_alive(Duration::from_secs(75))
    .bind(("0.0.0.0", port))?
    .run()
    .await
}

fn init_tracing() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt().with_env_filter(filter).init();
}
