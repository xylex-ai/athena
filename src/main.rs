use actix_cors::Cors;
use actix_files::NamedFile;
use actix_web::body::{BoxBody, EitherBody};
use actix_web::dev::{Service, ServiceResponse};
use actix_web::http::header;
use actix_web::{get, web, web::Path, App, HttpResponse, HttpServer, Responder};
use dotenv::dotenv;
use moka::future::Cache;
use serde_json::{json, Value};
use std::env::var;
use std::io::Result;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;
use tracing_subscriber::EnvFilter;
use web::Data;

pub type SharedCache = Arc<Mutex<Cache<String, Value>>>;

#[get("/ping")]
async fn status() -> impl Responder {
    let status_info: Value = json!({
        "status": "ok",
        "message": "Athena is healthy"
    });
    HttpResponse::Ok().json(status_info)
}
#[get("/docs")]
async fn redirect_to_docs() -> impl Responder {
    HttpResponse::Found()
        .header("Location", "/docs/index.html")
        .finish()
}

#[get("/docs/{filename:.*}")]
async fn serve_docs(path: web::Path<String>) -> impl Responder {
    let filename: String = path.into_inner();
    let file_path: String = if filename.is_empty() {
        "/home/floris-xlx/repos/athena/target/doc/athena_rs/index.html".to_string()
    } else {
        format!(
            "/home/floris-xlx/repos/athena/target/doc/athena_rs/{}",
            filename
        )
    };
    NamedFile::open_async(file_path).await.unwrap()
}

#[get("/static.files/{filename:.*}")]
async fn serve_static_files(path: Path<String>) -> impl Responder {
    let file_path: String = format!(
        "/home/floris-xlx/repos/athena/target/doc/static.files/{}",
        path.into_inner()
    );

    NamedFile::open_async(file_path).await.unwrap()
}

#[actix_web::main]
async fn main() -> Result<()> {
    init_tracing();
    dotenv().ok();
    let port: u16 = var("XLX_ATHENA_PORT")
        .unwrap_or("4052".to_string())
        .parse()
        .unwrap_or(4052);

    let cache: SharedCache = Arc::new(Mutex::new(
        Cache::builder()
            .time_to_live(Duration::from_secs(60))
            .build(),
    ));

    HttpServer::new(move || {
        let cors: Cors = Cors::default()
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
            // moka cache
            .app_data(Data::new(cache.clone()))
            .service(status)
            // docs
            .service(serve_docs)
            .service(serve_static_files)
            .service(redirect_to_docs)
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
}
