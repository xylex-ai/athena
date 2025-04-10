const TARGET_BASE_URL: &str = "https://db-suitsbooks-nl.xylex.cloud";

const HOST_DEXTER: &str = "db-dexter.xylex.cloud";
const TARGET_BASE_URL_DEXTER: &str = "https://athena.dexter.xylex.cloud";

pub fn determine_target_url(host: &str, path: &str) -> String {
    if host.contains(HOST_DEXTER) {
        format!("{}{}", TARGET_BASE_URL_DEXTER, path)
    } else {
        format!("{}{}", TARGET_BASE_URL, path)
    }
}
