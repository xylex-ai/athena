use actix_cors::Cors;
use actix_web::http::Method as ActixMethod;
use actix_web::body::{ BoxBody, EitherBody };
use actix_web::dev::{ Service, ServiceResponse };
use actix_web::http::{ header, StatusCode };
use actix_web::{ get, web, App, HttpRequest, HttpResponse, HttpServer, Responder };
use dotenv::dotenv;
use moka::future::Cache;
use reqwest::{ Client, Method };
use serde_json::{ json, Value };
use std::env::var;
use std::io::Result;
use std::sync::Arc;
use std::time::Duration;
use tracing::info;
use tracing_subscriber::EnvFilter;
use web::Data;

use crate::AppState;
const TARGET_BASE_URL: &str = "https://db-suitsbooks-nl.xylex.cloud";

const HOST_DEXTER: &str = "db-dexter.xylex.cloud";
const TARGET_BASE_URL_DEXTER: &str = "https://athena.dexter.xylex.cloud";

pub async fn proxy_request(
    req: HttpRequest,
    body: web::Bytes,
    app_state: Data<AppState>
) -> impl Responder {
    info!("Starting proxy request processing");
    let client: &Client = &app_state.client;
    let cache: &Arc<Cache<String, Value>> = &app_state.cache;
    let full_url: reqwest::Url = req.full_url();
    let full_url_path: &str = full_url.path();
    let query_params: &str = full_url.query().unwrap_or_default();
    let path: String = full_url_path.replacen("/rest/v1", "", 1);

    let host: &str = req
        .headers()
        .get(header::HOST)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();

    let target_url_repl: String = if host.contains(HOST_DEXTER) {
        format!("{}{}", TARGET_BASE_URL_DEXTER, path)
    } else {
        format!("{}{}", TARGET_BASE_URL, path)
    };

    let mut target_url: String = target_url_repl;
    // inject query params
    if !query_params.is_empty() {
        target_url.push_str("?");
        target_url.push_str(query_params);
    }

    info!("Target URL: {:#?}", target_url);

    // Extract the JWT token and remove the "Bearer " prefix
    let jwt_token: String = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .map(|value| value.to_string())
        .unwrap_or_default();

    info!("JWT Token: {:#?}", jwt_token);

    let cache_control_header: Option<header::HeaderValue> = req
        .headers()
        .get(header::CACHE_CONTROL)
        .cloned();

    let cachekey: String = format!("{}-{}-{}", req.method(), full_url, jwt_token)
        .replace('*', "_xXx_")
        .replace(' ', "_")
        .replace(':', "-")
        .replace('/', "_");

    if cache_control_header.as_ref().map_or(true, |h| h != "no-cache") {
        if let Some(cached_response) = cache.get(&cachekey).await {
            info!("Cache hit for key: {}", cachekey);
            return HttpResponse::Ok().json(cached_response);
        }
    }

    let reqwest_method: Method = match *req.method() {
        ActixMethod::GET => Method::GET,
        ActixMethod::POST => Method::POST,
        ActixMethod::PUT => Method::PUT,
        ActixMethod::DELETE => Method::DELETE,
        ActixMethod::PATCH => Method::PATCH,
        _ => Method::GET,
    };

    let mut client_req: reqwest::RequestBuilder = client.request(reqwest_method, &target_url);
    for (key, value) in req.headers().iter() {
        if key != header::HOST {
            let reqwest_key: reqwest::header::HeaderName = reqwest::header::HeaderName
                ::from_bytes(key.as_ref())
                .unwrap();
            let reqwest_value: reqwest::header::HeaderValue = reqwest::header::HeaderValue
                ::from_bytes(value.as_bytes())
                .unwrap();
            client_req = client_req.header(reqwest_key, reqwest_value);
        }
    }

    // Set the JWT token as the "apikey" header
    if !jwt_token.is_empty() {
        client_req = client_req.header("apikey", jwt_token);
    }

    match client_req.body(body).send().await {
        Ok(res) => {
            info!("Received response from target URL: {}", target_url);
            let status_code: StatusCode = StatusCode::from_u16(res.status().as_u16()).unwrap();
            let headers: reqwest::header::HeaderMap = res.headers().clone();
            let body_bytes: web::Bytes = res.bytes().await.unwrap_or_default();
            let json_body: Value = serde_json::from_slice(&body_bytes).unwrap_or(Value::Null);
            cache.insert(cachekey, json_body.clone()).await;
            let mut response: actix_web::HttpResponseBuilder = HttpResponse::build(status_code);

            for (key, value) in headers.iter() {
                if
                    ![
                        reqwest::header::CONTENT_ENCODING,
                        reqwest::header::CONTENT_LENGTH,
                        reqwest::header::TRANSFER_ENCODING,
                        reqwest::header::CONNECTION,
                    ].contains(key)
                {
                    let actix_key: header::HeaderName = actix_web::http::header::HeaderName
                        ::from_bytes(key.as_str().as_bytes())
                        .unwrap();
                    let actix_value: header::HeaderValue = actix_web::http::header::HeaderValue
                        ::from_bytes(value.as_bytes())
                        .unwrap();
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
        Err(e) => {
            info!("Error sending request to target URL: {}", e);
            HttpResponse::InternalServerError().finish()
        },
    }
}
