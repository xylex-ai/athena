use supabase_rs::SupabaseClient;
use serde_json::Value;
use tracing::{info, error};

use crate::data::athena_supabase;

pub async fn list_athena_router_entries() -> Result<Vec<Value>, String> {
    let client: SupabaseClient = athena_supabase().await;

    let data: Result<Vec<Value>, String> = client
        .select("pm_athena_router")
        .columns(["*"].to_vec())
        .execute()
        .await;

    match data {
        Ok(result) => Ok(result),
        Err(err) => Err(format!(
            "Failed to fetch list_athena_router_entries data: {:?}",
            err
        )),
    }
}