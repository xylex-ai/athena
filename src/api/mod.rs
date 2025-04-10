use actix_web::{ get, web, HttpResponse, Responder };
use moka::future::Cache;
use serde_json::Value;
use std::sync::Arc;
use tracing::info;
use web::Data;

// crate imports
use crate::data::athena_router::list_athena_router_entries;
use crate::ImmortalCache;


#[get("/athena/router")]
async fn athena_router(app_state: Data<ImmortalCache>) -> impl Responder {
    info!("Received request for athena router");

    let cache: &Arc<Cache<String, Value>> = &app_state.cache;
    let cache_key: String = "athena_router_entries".to_string();

    if let Some(cached_entries) = cache.get(&cache_key).await {
        info!("Cache hit for athena router entries");
        return HttpResponse::Ok().json(cached_entries);
    }

    match list_athena_router_entries().await {
        Ok(entries) => {
            cache.insert(cache_key.clone(), serde_json::Value::Array(entries.clone())).await;
            HttpResponse::Ok().json(entries)
        }
        Err(err) => HttpResponse::InternalServerError().body(err),
    }
}
