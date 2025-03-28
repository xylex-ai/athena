use actix_cors::Cors;
use actix_web::body::{BoxBody, EitherBody};
use actix_web::dev::{Service, ServiceResponse};
use actix_web::http::{header, StatusCode};
use actix_web::{get, web, App, HttpRequest, HttpResponse, HttpServer, Responder};
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

const TARGET_BASE_URL: &str = "https://db-suitsbooks-nl.xylex.cloud";

#[get("/")]
async fn ping() -> impl Responder {
    info!("Received ping request");
    HttpResponse::Ok().json(json!({"message": "pong"}))
}

fn is_last_char_slash(path: &str) -> bool {
    path.chars().last().unwrap_or_default() == '/'
}

async fn proxy_request(
    req: HttpRequest,
    body: web::Bytes,
    app_state: Data<AppState>,
) -> impl Responder {
    let client: &Client = &app_state.client;
    let cache: &Arc<Cache<String, Value>> = &app_state.cache;
    let full_url: reqwest::Url = req.full_url();
    let full_url_path: &str = full_url.path();
    let query_params: &str = full_url.query().unwrap_or_default();
    info!("full_url: {:#?}", full_url);
    info!("query_params: {:#?}", query_params);
    info!("full_url_path: {:#?}", full_url_path);
    let path: String = full_url_path.replacen("/rest/v1", "", 1);
    info!("path: {:#?}", path);

    let mut target_url: String = format!("{}{}", TARGET_BASE_URL, path);
    // inject query params
    if !query_params.is_empty() {
        target_url.push_str("?");
        target_url.push_str(query_params);
    }
  

    // add the slash if it's missing

    info!("target_url {:#?}", target_url);
    let jwt_token = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .map(|value| value.to_string())
        .unwrap_or_default();

    info!("jwt_token {:#?}", jwt_token);

    let cache_control_header: Option<header::HeaderValue> =
        req.headers().get(header::CACHE_CONTROL).cloned();

    info!("cache_control_header {:#?}", cache_control_header);

    let cachekey: String = format!("{}-{}-{}", req.method(), full_url, jwt_token)
        .replace('*', "_xXx_")
        .replace(' ', "_")
        .replace(':', "-")
        .replace('/', "_");

    info!("cachekey {:#?}", cachekey);

    if cache_control_header
        .as_ref()
        .map_or(true, |h| h != "no-cache")
    {
        if let Some(cached_response) = cache.get(&cachekey).await {
            return HttpResponse::Ok().json(cached_response);
        }
    }

    let reqwest_method: Method = match *req.method() {
        actix_web::http::Method::GET => Method::GET,
        actix_web::http::Method::POST => Method::POST,
        actix_web::http::Method::PUT => Method::PUT,
        actix_web::http::Method::DELETE => Method::DELETE,
        actix_web::http::Method::PATCH => Method::PATCH,
        _ => Method::GET,
    };

    info!("reqwest_method {:#?}", reqwest_method);

    let mut client_req: reqwest::RequestBuilder = client.request(reqwest_method, &target_url);
    for (key, value) in req.headers().iter() {
        if key != header::HOST {
            let reqwest_key = reqwest::header::HeaderName::from_bytes(key.as_ref()).unwrap();
            let reqwest_value = reqwest::header::HeaderValue::from_bytes(value.as_bytes()).unwrap();
            client_req = client_req.header(reqwest_key, reqwest_value);
        }
    }
    info!("client_req {:#?}", client_req);

    match client_req.body(body).send().await {
        Ok(res) => {
            let status_code = StatusCode::from_u16(res.status().as_u16()).unwrap();
            info!("status_code {:#?}", status_code);
            let headers = res.headers().clone();
            info!("headers {:#?}", headers);
            let body_bytes = res.bytes().await.unwrap_or_default();
            info!("body_bytes {:#?}", body_bytes);
            let json_body: Value = serde_json::from_slice(&body_bytes).unwrap_or(Value::Null);
            info!("json_body {:#?}", json_body);
            cache.insert(cachekey, json_body.clone()).await;
            let mut response: actix_web::HttpResponseBuilder = HttpResponse::build(status_code);

            for (key, value) in headers.iter() {
                if ![
                    reqwest::header::CONTENT_ENCODING,
                    reqwest::header::CONTENT_LENGTH,
                    reqwest::header::TRANSFER_ENCODING,
                    reqwest::header::CONNECTION,
                ]
                .contains(key)
                {
                    let actix_key: header::HeaderName =
                        actix_web::http::header::HeaderName::from_bytes(key.as_str().as_bytes())
                            .unwrap();
                    info!("actix_key {:#?}", actix_key);
                    let actix_value: header::HeaderValue =
                        actix_web::http::header::HeaderValue::from_bytes(value.as_bytes()).unwrap();
                    if actix_key == header::CONTENT_TYPE {
                        response.append_header((
                            actix_key,
                            header::HeaderValue::from_static("application/json"),
                        ));
                    } else {
                        response.append_header((actix_key, actix_value));
                    }
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
    let cache: Arc<Cache<String, Value>> = Arc::new(
        Cache::builder()
            .time_to_live(Duration::from_secs(60))
            .build(),
    );
    let client: Client = Client::builder()
        .pool_idle_timeout(Duration::from_secs(90))
        .build()
        .unwrap();
    let app_state: Data<AppState> = Data::new(AppState { cache, client });

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
