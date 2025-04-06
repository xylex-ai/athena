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

pub mod proxy_request;
pub mod drivers;

use crate::proxy_request::proxy_request;
pub struct AppState {
    cache: Arc<Cache<String, Value>>, // Removed Mutex for async-safe cache
    client: Client,
}

#[get("/")]
async fn ping() -> impl Responder {
    info!("Received ping request");
    HttpResponse::Ok().json(json!({"message": "pong"}))
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
