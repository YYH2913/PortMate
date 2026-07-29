use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub(crate) const MAX_JSON_RPC_BATCH_ITEMS: usize = 128;

#[derive(Debug, Deserialize)]
pub(crate) struct JsonRpcRequest {
    pub(crate) jsonrpc: String,
    pub(crate) id: Option<Value>,
    pub(crate) method: String,
    #[serde(default)]
    pub(crate) params: Value,
}

#[derive(Debug, Serialize)]
pub(crate) struct JsonRpcResponse {
    pub(crate) jsonrpc: &'static str,
    pub(crate) id: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) error: Option<JsonRpcError>,
}

#[derive(Debug, Serialize)]
pub(crate) struct JsonRpcError {
    code: i64,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    data: Option<Value>,
}

pub(crate) fn dispatch_json_rpc_value(
    value: Value,
    mut handle: impl FnMut(JsonRpcRequest) -> Result<Option<JsonRpcResponse>>,
) -> Result<Option<Value>> {
    if let Value::Array(items) = value {
        if items.is_empty() {
            return Ok(Some(serde_json::to_value(error(
                Value::Null,
                -32600,
                "an empty JSON-RPC batch is invalid",
            ))?));
        }
        if items.len() > MAX_JSON_RPC_BATCH_ITEMS {
            return Ok(Some(serde_json::to_value(error(
                Value::Null,
                -32600,
                format!("JSON-RPC batch exceeds the {MAX_JSON_RPC_BATCH_ITEMS}-item limit"),
            ))?));
        }
        let mut responses = Vec::with_capacity(items.len());
        for item in items {
            if let Some(response) = dispatch_one_json_rpc_value(item, &mut handle)? {
                responses.push(serde_json::to_value(response)?);
            }
        }
        return if responses.is_empty() {
            Ok(None)
        } else {
            Ok(Some(Value::Array(responses)))
        };
    }
    dispatch_one_json_rpc_value(value, &mut handle)?
        .map(serde_json::to_value)
        .transpose()
        .map_err(Into::into)
}

fn dispatch_one_json_rpc_value(
    value: Value,
    handle: &mut impl FnMut(JsonRpcRequest) -> Result<Option<JsonRpcResponse>>,
) -> Result<Option<JsonRpcResponse>> {
    let has_id = value.get("id").is_some();
    let id = value.get("id").cloned().unwrap_or(Value::Null);
    if has_id && !matches!(id, Value::Null | Value::Number(_) | Value::String(_)) {
        return Ok(Some(error(
            Value::Null,
            -32600,
            "JSON-RPC id must be a string, number, or null",
        )));
    }
    if value
        .get("params")
        .is_some_and(|params| !params.is_array() && !params.is_object())
    {
        return Ok(has_id.then(|| error(id, -32602, "JSON-RPC params must be an object or array")));
    }
    let mut request = match serde_json::from_value::<JsonRpcRequest>(value) {
        Ok(request) => request,
        Err(error_message) => return Ok(Some(error(id, -32600, error_message.to_string()))),
    };
    if has_id {
        request.id = Some(id.clone());
    }
    match handle(request) {
        Ok(response) => Ok(response),
        Err(error_message) if has_id => Ok(Some(error(id, -32603, error_message.to_string()))),
        Err(_) => Ok(None),
    }
}

pub(crate) fn error(id: Value, code: i64, message: impl Into<String>) -> JsonRpcResponse {
    JsonRpcResponse {
        jsonrpc: "2.0",
        id,
        result: None,
        error: Some(JsonRpcError {
            code,
            message: message.into(),
            data: None,
        }),
    }
}
