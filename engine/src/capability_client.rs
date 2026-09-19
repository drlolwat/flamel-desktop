use rmcp::{
    ServiceExt,
    model::{CallToolRequestParams, ClientInfo},
    transport::{
        StreamableHttpClientTransport, TokioChildProcess,
        streamable_http_client::StreamableHttpClientTransportConfig,
    },
};
use serde_json::Value;
use std::time::{Duration, Instant};
use tokio_util::sync::CancellationToken;

const REQUEST_TIMEOUT: Duration = Duration::from_secs(45);

#[derive(Debug)]
pub struct CapabilityResult {
    pub tools: Vec<Value>,
    pub resources: Vec<Value>,
    pub prompts: Vec<Value>,
    pub output: Option<Value>,
    pub latency_ms: i64,
    pub process_id: Option<i64>,
}

fn arguments(value: &Value) -> Result<serde_json::Map<String, Value>, String> {
    match value {
        Value::Object(map) => Ok(map.clone()),
        Value::Null => Ok(serde_json::Map::new()),
        _ => Err("Capability tool arguments must be a JSON object".into()),
    }
}

async fn bounded<T>(
    cancellation: &CancellationToken,
    future: impl std::future::Future<Output = Result<T, rmcp::service::ServiceError>>,
) -> Result<T, String> {
    tokio::select! {
        _ = cancellation.cancelled() => Err("Capability task cancelled".into()),
        result = tokio::time::timeout(REQUEST_TIMEOUT, future) => match result {
            Ok(Ok(value)) => Ok(value),
            Ok(Err(error)) => Err(format!("MCP request failed: {error}")),
            Err(_) => Err("MCP request timed out after 45 seconds".into()),
        },
    }
}

pub async fn run_stdio(
    command: &str,
    args: &[String],
    call: Option<(&str, &Value)>,
    cancellation: &CancellationToken,
) -> Result<CapabilityResult, String> {
    let started = Instant::now();
    let mut command_builder = tokio::process::Command::new(command);
    command_builder.args(args);
    let transport = TokioChildProcess::new(command_builder)
        .map_err(|error| format!("Could not start integration: {error}"))?;
    let process_id = transport.id().map(i64::from);
    let client = tokio::select! {
        _ = cancellation.cancelled() => return Err("Capability task cancelled".into()),
        result = tokio::time::timeout(REQUEST_TIMEOUT, ClientInfo::default().serve(transport)) => {
            result.map_err(|_| "MCP initialization timed out".to_string())?
                .map_err(|error| format!("MCP initialization failed: {error}"))?
        }
    };

    let result = async {
        let listed = bounded(cancellation, client.list_all_tools()).await?;
        let tools = listed
            .into_iter()
            .map(|tool| serde_json::to_value(tool).map_err(|error| error.to_string()))
            .collect::<Result<Vec<_>, _>>()?;
        let resources = bounded(cancellation, client.list_all_resources())
            .await
            .unwrap_or_default()
            .into_iter()
            .filter_map(|item| serde_json::to_value(item).ok())
            .collect();
        let prompts = bounded(cancellation, client.list_all_prompts())
            .await
            .unwrap_or_default()
            .into_iter()
            .filter_map(|item| serde_json::to_value(item).ok())
            .collect();
        let output = if let Some((name, input)) = call {
            let request =
                CallToolRequestParams::new(name.to_string()).with_arguments(arguments(input)?);
            Some(
                serde_json::to_value(bounded(cancellation, client.call_tool(request)).await?)
                    .map_err(|error| error.to_string())?,
            )
        } else {
            None
        };
        Ok::<_, String>((tools, resources, prompts, output))
    }
    .await;
    let _ = client.cancel().await;
    let (tools, resources, prompts, output) = result?;
    Ok(CapabilityResult {
        tools,
        resources,
        prompts,
        output,
        latency_ms: started.elapsed().as_millis() as i64,
        process_id,
    })
}

pub async fn run_remote(
    url: &str,
    call: Option<(&str, &Value)>,
    bearer: Option<&str>,
    cancellation: &CancellationToken,
) -> Result<CapabilityResult, String> {
    let started = Instant::now();
    // rmcp's reqwest client enables rustls without choosing a crypto provider.
    // Install the same ring provider used by our other TLS clients before its
    // first Client::new(); keep a provider already installed by the host.
    let _ = rustls::crypto::ring::default_provider().install_default();
    let mut config = StreamableHttpClientTransportConfig::with_uri(url.to_string());
    if let Some(token) = bearer {
        config = config.auth_header(token.to_string());
    }
    let transport = StreamableHttpClientTransport::from_config(config);
    let client = tokio::select! {
        _ = cancellation.cancelled() => return Err("Capability task cancelled".into()),
        result = tokio::time::timeout(REQUEST_TIMEOUT, ClientInfo::default().serve(transport)) => {
            result.map_err(|_| "MCP initialization timed out".to_string())?
                .map_err(|error| format!("MCP initialization failed: {error}"))?
        }
    };
    let result = async {
        let listed = bounded(cancellation, client.list_all_tools()).await?;
        let tools = listed
            .into_iter()
            .map(|tool| serde_json::to_value(tool).map_err(|error| error.to_string()))
            .collect::<Result<Vec<_>, _>>()?;
        let resources = bounded(cancellation, client.list_all_resources())
            .await
            .unwrap_or_default()
            .into_iter()
            .filter_map(|item| serde_json::to_value(item).ok())
            .collect();
        let prompts = bounded(cancellation, client.list_all_prompts())
            .await
            .unwrap_or_default()
            .into_iter()
            .filter_map(|item| serde_json::to_value(item).ok())
            .collect();
        let output = if let Some((name, input)) = call {
            let request =
                CallToolRequestParams::new(name.to_string()).with_arguments(arguments(input)?);
            Some(
                serde_json::to_value(bounded(cancellation, client.call_tool(request)).await?)
                    .map_err(|error| error.to_string())?,
            )
        } else {
            None
        };
        Ok::<_, String>((tools, resources, prompts, output))
    }
    .await;
    let _ = client.cancel().await;
    let (tools, resources, prompts, output) = result?;
    Ok(CapabilityResult {
        tools,
        resources,
        prompts,
        output,
        latency_ms: started.elapsed().as_millis() as i64,
        process_id: None,
    })
}
