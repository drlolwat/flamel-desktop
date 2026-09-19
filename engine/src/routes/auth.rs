use axum::Json;
use serde_json::{Value, json};
pub async fn handle_setup_required() -> Json<Value> {
    Json(
        json!({"required":false,"local":true,"flamel_registered":false,"capabilities":{"localWorkspace":true,"networkHosting":false,"remoteWorkers":false,"localAccounts":false,"sharing":false,"serverAdministration":false}}),
    )
}
