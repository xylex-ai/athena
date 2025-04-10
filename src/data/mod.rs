use dotenv::dotenv;
use supabase_rs::SupabaseClient;
use dotenv::var;


pub mod athena_router;

pub async fn athena_supabase() -> SupabaseClient {
    dotenv().ok(); // Load the .env file

    let supabase_client: SupabaseClient = SupabaseClient::new(
        var("XLX_ATHENA_SUPABASE_URL").unwrap(),
        var("XLX_ATHENA_SUPABASE_KEY").unwrap()
    ).unwrap();

    supabase_client
}
