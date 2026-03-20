use serde::{Deserialize, Serialize};

// Standard JSON-RPC 2.0 error codes
pub const PARSE_ERROR: i32 = -32700;
pub const INVALID_REQUEST: i32 = -32600;
pub const METHOD_NOT_FOUND: i32 = -32601;
pub const INVALID_PARAMS: i32 = -32602;
pub const INTERNAL_ERROR: i32 = -32603;

// A2A-specific error codes
pub const TASK_NOT_FOUND: i32 = -32001;
pub const TASK_NOT_CANCELABLE: i32 = -32002;
pub const PUSH_NOTIFICATION_NOT_SUPPORTED: i32 = -32003;
pub const UNSUPPORTED_OPERATION: i32 = -32004;
pub const CONTENT_TYPE_NOT_SUPPORTED: i32 = -32005;
pub const INVALID_AGENT_RESPONSE: i32 = -32006;

/// JSON-RPC 2.0 request/notification ID.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum JsonRpcId {
    Number(i64),
    String(String),
}

/// JSON-RPC 2.0 request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,
    pub method: String,
    #[serde(default)]
    pub params: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<JsonRpcId>,
}

/// JSON-RPC 2.0 response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcResponse {
    pub jsonrpc: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
    pub id: Option<JsonRpcId>,
}

impl JsonRpcResponse {
    /// Create a success response.
    pub fn success(id: Option<JsonRpcId>, result: serde_json::Value) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            result: Some(result),
            error: None,
            id,
        }
    }

    /// Create an error response.
    pub fn error(id: Option<JsonRpcId>, error: JsonRpcError) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            result: None,
            error: Some(error),
            id,
        }
    }
}

/// JSON-RPC 2.0 error object.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcError {
    pub code: i32,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}

impl JsonRpcError {
    /// Create an error from a standard code with default message.
    pub fn from_code(code: i32) -> Self {
        let message = match code {
            PARSE_ERROR => "Parse error",
            INVALID_REQUEST => "Invalid Request",
            METHOD_NOT_FOUND => "Method not found",
            INVALID_PARAMS => "Invalid params",
            INTERNAL_ERROR => "Internal error",
            TASK_NOT_FOUND => "Task not found",
            TASK_NOT_CANCELABLE => "Task not cancelable",
            PUSH_NOTIFICATION_NOT_SUPPORTED => "Push notification not supported",
            UNSUPPORTED_OPERATION => "Unsupported operation",
            CONTENT_TYPE_NOT_SUPPORTED => "Incompatible content types",
            INVALID_AGENT_RESPONSE => "Invalid agent response",
            _ => "Unknown error",
        };
        Self {
            code,
            message: message.to_string(),
            data: None,
        }
    }

    /// Create an error with a custom message.
    pub fn with_message(code: i32, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            data: None,
        }
    }
}

// Note: PartialEq not derived on JsonRpcError/JsonRpcResponse, so tests compare fields directly.

/// A2A protocol methods.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum A2aMethod {
    MessageSend,
    MessageStream,
    TasksGet,
    TasksCancel,
    TasksResubscribe,
    PushNotificationConfigSet,
    PushNotificationConfigGet,
    PushNotificationConfigList,
    PushNotificationConfigDelete,
}

impl A2aMethod {
    /// Parse a method string into an A2aMethod.
    pub fn parse(method: &str) -> Option<Self> {
        match method {
            "message/send" => Some(Self::MessageSend),
            "message/stream" => Some(Self::MessageStream),
            "tasks/get" => Some(Self::TasksGet),
            "tasks/cancel" => Some(Self::TasksCancel),
            "tasks/resubscribe" => Some(Self::TasksResubscribe),
            "tasks/pushNotificationConfig/set" => Some(Self::PushNotificationConfigSet),
            "tasks/pushNotificationConfig/get" => Some(Self::PushNotificationConfigGet),
            "tasks/pushNotificationConfig/list" => Some(Self::PushNotificationConfigList),
            "tasks/pushNotificationConfig/delete" => Some(Self::PushNotificationConfigDelete),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_jsonrpc_request_from_json() {
        let json = r#"{
            "jsonrpc": "2.0",
            "method": "message/send",
            "params": {"message": {"messageId": "m1", "role": "user", "parts": []}},
            "id": 42
        }"#;
        let req: JsonRpcRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.jsonrpc, "2.0");
        assert_eq!(req.method, "message/send");
        assert_eq!(req.id, Some(JsonRpcId::Number(42)));
        assert!(req.params.is_object());
    }

    #[test]
    fn parse_jsonrpc_request_string_id() {
        let json = r#"{"jsonrpc":"2.0","method":"tasks/get","params":{},"id":"abc-123"}"#;
        let req: JsonRpcRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.id, Some(JsonRpcId::String("abc-123".to_string())));
    }

    #[test]
    fn parse_jsonrpc_request_null_id() {
        // With serde(untagged) on JsonRpcId and Option<JsonRpcId>, "id": null deserializes as None
        let json = r#"{"jsonrpc":"2.0","method":"tasks/get","params":{},"id":null}"#;
        let req: JsonRpcRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.id, None);
    }

    #[test]
    fn parse_jsonrpc_request_no_id() {
        // Notifications have no id field
        let json = r#"{"jsonrpc":"2.0","method":"tasks/get","params":{}}"#;
        let req: JsonRpcRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.id, None);
    }

    #[test]
    fn parse_jsonrpc_request_default_params() {
        let json = r#"{"jsonrpc":"2.0","method":"tasks/get"}"#;
        let req: JsonRpcRequest = serde_json::from_str(json).unwrap();
        assert!(req.params.is_null());
    }

    #[test]
    fn jsonrpc_response_success() {
        let id = Some(JsonRpcId::Number(1));
        let resp = JsonRpcResponse::success(id, serde_json::json!({"status": "ok"}));
        assert_eq!(resp.jsonrpc, "2.0");
        assert!(resp.result.is_some());
        assert!(resp.error.is_none());
        assert_eq!(resp.result.unwrap()["status"], "ok");
    }

    #[test]
    fn jsonrpc_response_error() {
        let id = Some(JsonRpcId::String("req-1".to_string()));
        let err = JsonRpcError::from_code(TASK_NOT_FOUND);
        let resp = JsonRpcResponse::error(id, err);
        assert_eq!(resp.jsonrpc, "2.0");
        assert!(resp.result.is_none());
        assert!(resp.error.is_some());
        let e = resp.error.unwrap();
        assert_eq!(e.code, TASK_NOT_FOUND);
        assert_eq!(e.message, "Task not found");
    }

    #[test]
    fn jsonrpc_response_round_trip() {
        let resp =
            JsonRpcResponse::success(Some(JsonRpcId::Number(7)), serde_json::json!({"data": 42}));
        let json = serde_json::to_string(&resp).unwrap();
        let parsed: JsonRpcResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.jsonrpc, "2.0");
        assert_eq!(parsed.result.unwrap()["data"], 42);
        assert!(parsed.error.is_none());
    }

    #[test]
    fn jsonrpc_error_from_code_all_standard() {
        let cases = [
            (PARSE_ERROR, "Parse error"),
            (INVALID_REQUEST, "Invalid Request"),
            (METHOD_NOT_FOUND, "Method not found"),
            (INVALID_PARAMS, "Invalid params"),
            (INTERNAL_ERROR, "Internal error"),
            (TASK_NOT_FOUND, "Task not found"),
            (TASK_NOT_CANCELABLE, "Task not cancelable"),
            (
                PUSH_NOTIFICATION_NOT_SUPPORTED,
                "Push notification not supported",
            ),
            (UNSUPPORTED_OPERATION, "Unsupported operation"),
            (CONTENT_TYPE_NOT_SUPPORTED, "Incompatible content types"),
            (INVALID_AGENT_RESPONSE, "Invalid agent response"),
        ];
        for (code, expected_msg) in &cases {
            let err = JsonRpcError::from_code(*code);
            assert_eq!(err.code, *code);
            assert_eq!(err.message, *expected_msg, "code {code}");
            assert!(err.data.is_none());
        }
    }

    #[test]
    fn jsonrpc_error_from_code_unknown() {
        let err = JsonRpcError::from_code(-99999);
        assert_eq!(err.code, -99999);
        assert_eq!(err.message, "Unknown error");
    }

    #[test]
    fn jsonrpc_error_with_message() {
        let err = JsonRpcError::with_message(INTERNAL_ERROR, "something broke");
        assert_eq!(err.code, INTERNAL_ERROR);
        assert_eq!(err.message, "something broke");
    }

    #[test]
    fn a2a_method_parse_all() {
        let cases = [
            ("message/send", A2aMethod::MessageSend),
            ("message/stream", A2aMethod::MessageStream),
            ("tasks/get", A2aMethod::TasksGet),
            ("tasks/cancel", A2aMethod::TasksCancel),
            ("tasks/resubscribe", A2aMethod::TasksResubscribe),
            (
                "tasks/pushNotificationConfig/set",
                A2aMethod::PushNotificationConfigSet,
            ),
            (
                "tasks/pushNotificationConfig/get",
                A2aMethod::PushNotificationConfigGet,
            ),
            (
                "tasks/pushNotificationConfig/list",
                A2aMethod::PushNotificationConfigList,
            ),
            (
                "tasks/pushNotificationConfig/delete",
                A2aMethod::PushNotificationConfigDelete,
            ),
        ];
        for (method_str, expected) in &cases {
            let parsed = A2aMethod::parse(method_str);
            assert_eq!(parsed, Some(expected.clone()), "parse({method_str})");
        }
    }

    #[test]
    fn a2a_method_parse_unknown() {
        assert_eq!(A2aMethod::parse("unknown/method"), None);
        assert_eq!(A2aMethod::parse(""), None);
        assert_eq!(A2aMethod::parse("message/send/extra"), None);
    }
}
