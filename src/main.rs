use actix_cors::Cors;
use actix_web::body::{BoxBody, EitherBody};
use actix_web::dev::{Service, ServiceResponse};
use actix_web::http::{header, StatusCode};
use actix_web::{get, web, App, HttpRequest, HttpResponse, HttpServer, Responder};
use dotenv::dotenv;
use futures::TryStreamExt;
use moka::future::Cache;
use reqwest::{Client, Method};
use scylla::client::session::Session;
use scylla::client::session_builder::SessionBuilder;
use serde_json::{json, Value};
use std::env::var;
use std::error::Error;

use std::sync::Arc;
use std::time::Duration;
use tracing::info;
use tracing_subscriber::EnvFilter;
use web::Data;

pub mod drivers;
pub mod proxy_request;

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
async fn main() -> std::io::Result<()> {
    dotenv().ok();
    init_tracing();
    let port: u16 = var("XLX_ATHENA_PORT")
        .unwrap_or_else(|_| "4053".to_string())
        .parse()
        .unwrap_or(4053);

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
            .service(scylla_query_endpoint)
            .service(scylla_query_tables)
            .service(scylla_query_columns)
            .service(scylla_list_tables_endpoint)
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

#[get("/scylla")]
async fn scylla_query_endpoint() -> impl Responder {
    match scylla_query().await {
        Ok(_) => HttpResponse::Ok().body("Scylla query executed successfully."),
        Err(e) => {
            HttpResponse::InternalServerError().body(format!("Error executing Scylla query: {}", e))
        }
    }
}

#[get("/scylla/tables")]
async fn scylla_query_tables() -> impl Responder {
    match scylla_list_tables().await {
        Ok(tables) => HttpResponse::Ok().json(tables),
        Err(e) => {
            HttpResponse::InternalServerError().json(json!({"error": format!("Error listing Scylla tables: {}", e)}))
        }
    }
}

#[get("/scylla/columns")]
async fn scylla_query_columns(req: HttpRequest) -> impl Responder {
    if let Some(table_name) = req.query_string().split('=').nth(1) {
        match scylla_list_columns(table_name).await {
            Ok(columns) => HttpResponse::Ok().json(columns),
            Err(e) => HttpResponse::InternalServerError().json(json!({"error": format!("Error listing columns for table {}: {}", table_name, e)})),
        }
    } else {
        HttpResponse::BadRequest().body("Missing table_name query parameter")
    }
}


#[get("/scylla/list_tables")]
async fn scylla_list_tables_endpoint() -> impl Responder {
    match get_all_tables_and_columns().await {
        Ok(tables) => HttpResponse::Ok().json(tables),
        Err(e) => {
            HttpResponse::InternalServerError().json(json!({"error": format!("Error listing Scylla tables: {}", e)}))
        }
    }
}



async fn scylla_query() -> Result<(), Box<dyn Error>> {
    // Create a new Session which connects to node at 127.0.0.1:9042
    // (or SCYLLA_URI if specified)
    let uri = std::env::var("SCYLLA_URI").unwrap_or_else(|_| "127.0.0.1:9042".to_string());

    let session: Session = SessionBuilder::new().known_node(uri).build().await?;

    // Create the users keyspace and table with user_id as UUID
    session
        .query_unpaged(
            "CREATE KEYSPACE IF NOT EXISTS users WITH REPLICATION = \
            {'class' : 'NetworkTopologyStrategy', 'replication_factor' : 1}",
            &[],
        )
        .await?;

    session
        .query_unpaged(
            "CREATE TABLE IF NOT EXISTS users.users (user_id uuid primary key)",
            &[],
        )
        .await?;

    // Insert a value into the table
    let to_insert: uuid::Uuid = uuid::Uuid::new_v4();
    session
        .query_unpaged("INSERT INTO users.users (user_id) VALUES(?)", (to_insert,))
        .await?;

    // Query rows from the table and print them
    let mut iter = session
        .query_iter("SELECT user_id FROM users.users", &[])
        .await?
        .rows_stream::<(uuid::Uuid,)>()?;
    while let Some(read_row) = iter.try_next().await? {
        println!("Read a value from row: {}", read_row.0);
    }

    Ok(())
}

async fn scylla_list_tables() -> Result<serde_json::Value, Box<dyn std::error::Error>> {
    let uri = std::env::var("SCYLLA_URI").unwrap_or_else(|_| "127.0.0.1:9042".to_string());
    let session: Session = SessionBuilder::new()
        .known_node(uri)
        .build()
        .await?;

    let query = "SELECT table_name FROM system_schema.tables WHERE keyspace_name = 'users'";
    let mut iter = session
        .query_iter(query, &[])
        .await?
        .rows_stream::<(String,)>()?;

    let mut tables = Vec::new();
    while let Some(row) = iter.try_next().await? {
        tables.push(row.0);
    }

    let json_result = serde_json::json!({ "tables": tables });
    Ok(json_result)
}


async fn scylla_list_columns(table_name: &str) -> Result<serde_json::Value, Box<dyn std::error::Error>> {
    let uri = std::env::var("SCYLLA_URI").unwrap_or_else(|_| "127.0.0.1:9042".to_string());
    let session: Session = SessionBuilder::new()
        .known_node(uri)
        .build()
        .await?;

    let query = format!(
        "SELECT column_name FROM system_schema.columns WHERE keyspace_name = 'users' AND table_name = '{}'",
        table_name
    );
    let mut iter = session
        .query_iter(query.as_str(), &[])
        .await?
        .rows_stream::<(String,)>()?;

    let mut columns = Vec::new();
    while let Some(row) = iter.try_next().await? {
        columns.push(row.0);
    }

    let json_result = serde_json::json!({ "columns": columns });
    Ok(json_result)
}


async fn get_all_tables_and_columns() -> Result<serde_json::Value, Box<dyn std::error::Error>> {
    let uri = std::env::var("SCYLLA_URI").unwrap_or_else(|_| "127.0.0.1:9042".to_string());
    let session: Session = SessionBuilder::new()
        .known_node(uri)
        .build()
        .await?;

    let tables_query = "SELECT table_name FROM system_schema.tables WHERE keyspace_name = 'users'";
    let mut tables_iter = session
        .query_iter(tables_query, &[])
        .await?
        .rows_stream::<(String,)>()?;

    let mut data = Vec::new();

    while let Some((table_name,)) = tables_iter.try_next().await? {
        let columns_query = format!(
            "SELECT column_name, type FROM system_schema.columns WHERE keyspace_name = 'users' AND table_name = '{}'",
            table_name
        );
        let mut columns_iter = session
            .query_iter(columns_query.as_str(), &[])
            .await?
            .rows_stream::<(String, String)>()?;

        let mut columns = Vec::new();
        while let Some((column_name, data_type)) = columns_iter.try_next().await? {
            columns.push(serde_json::json!({
                "column_name": column_name,
                "data_type": data_type
            }));
        }

        data.push(serde_json::json!({
            "table_name": table_name,
            "columns": columns
        }));
    }

    let json_result = serde_json::json!({
        "data": data,
        "db_name": "suitsbooks_nl",
        "duration": 594, // Placeholder for actual duration calculation
        "message": "Successfully fetched tables",
        "status": "success"
    });

    Ok(json_result)
}
