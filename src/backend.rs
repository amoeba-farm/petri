use reqwest::StatusCode;
use reqwest::blocking::{Client, Response};
use reqwest::header::{
    ACCEPT, CACHE_CONTROL, CONTENT_LENGTH, HeaderMap, HeaderName, HeaderValue, RETRY_AFTER,
};
use serde::Serialize;
use serde_json::{Map, Value};
use std::env;
use std::error::Error;
use std::fmt::{Display, Formatter, Write as _};
use std::io::Read;
use std::thread;
use std::time::Duration;

pub(crate) fn current_backend_payload(payload: Value) -> Result<Value, CliError> {
    crate::chain_identity::validate_current_backend_envelope(&payload)?;
    Ok(payload)
}

pub(crate) fn unwrap_data<'a>(payload: &'a Value) -> &'a Value {
    value_at_key(payload, &["data"]).unwrap_or(payload)
}

const REQUEST_TIMEOUT_SECONDS: u64 = 15;
const GET_ATTEMPTS: usize = 3;
const GET_RETRY_BASE_DELAY_MS: u64 = 100;
const REQUEST_TIMEOUT_MS_ENV: &str = "PETRI_BACKEND_TIMEOUT_MS";
const AMEBA_REQUEST_TIMEOUT_MS_ENV: &str = "AMEBA_BACKEND_TIMEOUT_MS";
const AMEBA_CLIENT_HEADER: &str = "x-ameba-client";
const PETRI_CLIENT_ID: &str = "petri-cli";
const MAX_BACKEND_RESPONSE_BYTES: usize = 2 * 1024 * 1024;
const MAX_ERROR_DISPLAY_CHARS: usize = 4 * 1024;
const MAX_TERMINAL_JSON_BYTES: usize = MAX_BACKEND_RESPONSE_BYTES * 2;
pub(crate) const MAX_SAFE_JSON_INTEGER: u64 = 9_007_199_254_740_991;

#[derive(Debug)]
pub struct CliError {
    message: String,
    current_state_wait: bool,
    code: String,
    category: String,
    retryable: bool,
    operation_id: Option<String>,
    signature: Option<String>,
}

impl CliError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            current_state_wait: false,
            code: "PETRI_VALIDATION_FAILED".into(),
            category: "validation".into(),
            retryable: false,
            operation_id: None,
            signature: None,
        }
    }

    pub fn current_state_wait(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            current_state_wait: true,
            code: "CURRENT_STATE_UNAVAILABLE".into(),
            category: "unavailable".into(),
            retryable: true,
            operation_id: None,
            signature: None,
        }
    }

    pub fn is_current_state_wait(&self) -> bool {
        self.current_state_wait
    }

    pub fn coded(
        code: impl Into<String>,
        category: &str,
        message: impl Into<String>,
        retryable: bool,
    ) -> Self {
        Self {
            code: code.into(),
            category: category.into(),
            message: message.into(),
            retryable,
            current_state_wait: false,
            operation_id: None,
            signature: None,
        }
    }
    pub fn uncertain(message: impl Into<String>, operation_id: &str, signature: &str) -> Self {
        Self {
            code: "OPERATION_TRANSPORT_UNCERTAIN".into(),
            category: "pending".into(),
            message: message.into(),
            retryable: false,
            current_state_wait: true,
            operation_id: Some(operation_id.into()),
            signature: Some(signature.into()),
        }
    }
    pub fn confirmed_failure(
        message: impl Into<String>,
        operation_id: &str,
        signature: &str,
    ) -> Self {
        Self {
            code: "OPERATION_FAILED_ON_CHAIN".into(),
            category: "execution_failed".into(),
            message: message.into(),
            retryable: false,
            current_state_wait: false,
            operation_id: Some(operation_id.into()),
            signature: Some(signature.into()),
        }
    }
    pub fn json(&self) -> Value {
        serde_json::json!({"ok":false,"error":{"code":self.code,"category":self.category,
            "message":terminal_safe_text(&self.message),"retryable":self.retryable,"operationId":self.operation_id,
            "signature":self.signature,"nextStep":if self.operation_id.is_some(){"Recover this operation; never automatically resubmit."}else{"Correct the input or refresh the affected state."}}})
    }
    pub fn exit_code(&self) -> i32 {
        if self.current_state_wait || self.category == "pending" {
            75
        } else if self.category == "denied" {
            77
        } else if self.category == "unavailable" {
            69
        } else if self.category == "execution_failed" {
            70
        } else {
            1
        }
    }
}

impl Display for CliError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", terminal_safe_text(&self.message))
    }
}

impl Error for CliError {}

pub struct BackendClient {
    base_url: String,
    http: Client,
}

impl BackendClient {
    pub fn new(base_url: impl Into<String>) -> Result<Self, CliError> {
        let base_url = validate_backend_url(&base_url.into())?;
        let http = pinned_blocking_http_client_builder()
            .timeout(request_timeout())
            .user_agent(PETRI_CLIENT_ID)
            .default_headers(default_headers())
            .build()
            .map_err(|error| CliError::new(format!("failed to build HTTP client: {error}")))?;

        Ok(Self { base_url, http })
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    pub fn get(&self, path: &str) -> Result<Value, CliError> {
        let url = self.url(path);
        for attempt in 1..=GET_ATTEMPTS {
            match self.http.get(&url).send() {
                Ok(response) => {
                    let status = response.status();
                    if retryable_get_status(status) && attempt < GET_ATTEMPTS {
                        thread::sleep(get_retry_delay(attempt));
                        continue;
                    }
                    return decode_response(response, path).map_err(|mut error| {
                        if attempt > 1 {
                            error.message = format!("{} after {attempt} attempts", error.message);
                        }
                        error
                    });
                }
                Err(error) => {
                    let retryable = error.is_timeout() || error.is_connect() || error.is_body();
                    if retryable && attempt < GET_ATTEMPTS {
                        thread::sleep(get_retry_delay(attempt));
                        continue;
                    }
                    let attempts = if attempt > 1 {
                        format!(" after {attempt} attempts")
                    } else {
                        String::new()
                    };
                    return Err(CliError::coded(
                        "BACKEND_UNAVAILABLE",
                        "unavailable",
                        format!("GET {path} failed{attempts}: {error}"),
                        retryable,
                    ));
                }
            }
        }
        unreachable!("bounded GET retry loop always returns")
    }

    pub fn post_json_with_current_state_retry<T: Serialize>(
        &self,
        path: &str,
        payload: &T,
    ) -> Result<Value, CliError> {
        for attempt in 0..=1 {
            let response = self
                .http
                .post(self.url(path))
                .json(payload)
                .timeout(request_timeout())
                .send()
                .map_err(|error| CliError::new(format!("POST {path} failed: {error}")))?;
            let status = response.status();
            let current_state_headers_are_exact =
                header_is_exactly_once(response.headers(), &CACHE_CONTROL, "no-store")
                    && !response.headers().contains_key(RETRY_AFTER);
            let body = read_bounded_response(response, path)?;
            let parsed = parse_response_body(&body);
            if current_state_unavailable(status, current_state_headers_are_exact, &parsed) {
                if attempt == 0 {
                    thread::sleep(Duration::from_millis(GET_RETRY_BASE_DELAY_MS));
                    continue;
                }
                return Err(CliError::current_state_wait(format!(
                    "Waiting for current finalized state; Amoeba returned CURRENT_STATE_UNAVAILABLE after one bounded retry ({path}). Try again shortly."
                )));
            }
            return decode_response_parts(status, parsed, path);
        }
        unreachable!("bounded current-state retry loop always returns")
    }

    /// Sends one state-changing Amoeba request exactly once. Callers must use
    /// the operation-status route to resolve an ambiguous transport outcome;
    /// this method never retries a mutation.
    pub fn post_json<T: Serialize>(&self, path: &str, payload: &T) -> Result<Value, CliError> {
        let response = self
            .http
            .post(self.url(path))
            .json(payload)
            .timeout(request_timeout())
            .send()
            .map_err(|error| CliError::new(format!("POST {path} failed: {error}")))?;
        decode_response(response, path)
    }

    fn url(&self, path: &str) -> String {
        format!("{}/{}", self.base_url, path.trim_start_matches('/'))
    }
}

pub(crate) fn pinned_blocking_http_client_builder() -> reqwest::blocking::ClientBuilder {
    configure_pinned_blocking_http_client_builder(Client::builder())
}

fn configure_pinned_blocking_http_client_builder(
    builder: reqwest::blocking::ClientBuilder,
) -> reqwest::blocking::ClientBuilder {
    // Origin validation must bind the actual peer. Ambient proxy settings
    // could otherwise route even an accepted loopback URL to a remote proxy.
    // Disable protocol retries as well so each `.send()` is one wire attempt;
    // callers own any explicit, typed read retry policy.
    builder
        .redirect(reqwest::redirect::Policy::none())
        .no_proxy()
        .retry(reqwest::retry::never())
}

fn default_headers() -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(
        HeaderName::from_static(AMEBA_CLIENT_HEADER),
        HeaderValue::from_static(PETRI_CLIENT_ID),
    );
    headers.insert(ACCEPT, HeaderValue::from_static("application/json"));
    headers
}

fn header_is_exactly_once(headers: &HeaderMap, name: &HeaderName, expected: &str) -> bool {
    let mut values = headers.get_all(name).iter();
    values.next().and_then(|value| value.to_str().ok()) == Some(expected) && values.next().is_none()
}

fn validate_backend_url(raw: &str) -> Result<String, CliError> {
    crate::petri_config::normalize_amoeba_backend_url(raw).map_err(CliError::new)
}

fn request_timeout() -> Duration {
    timeout_from_millis_env(
        env::var(REQUEST_TIMEOUT_MS_ENV)
            .ok()
            .or_else(|| env::var(AMEBA_REQUEST_TIMEOUT_MS_ENV).ok())
            .as_deref(),
    )
}

fn timeout_from_millis_env(raw: Option<&str>) -> Duration {
    raw.and_then(|value| value.trim().parse::<u64>().ok())
        .filter(|millis| *millis > 0)
        .map(Duration::from_millis)
        .unwrap_or_else(|| Duration::from_secs(REQUEST_TIMEOUT_SECONDS))
}

fn retryable_get_status(status: StatusCode) -> bool {
    matches!(status.as_u16(), 408 | 425 | 429 | 500 | 502 | 503 | 504)
}

fn get_retry_delay(attempt: usize) -> Duration {
    Duration::from_millis(
        GET_RETRY_BASE_DELAY_MS.saturating_mul(1_u64 << attempt.saturating_sub(1).min(5)),
    )
}

fn decode_response(response: Response, path: &str) -> Result<Value, CliError> {
    let status = response.status();
    let body = read_bounded_response(response, path)?;
    decode_response_parts(status, parse_response_body(&body), path)
}

fn read_bounded_response(mut response: Response, path: &str) -> Result<String, CliError> {
    if response
        .headers()
        .get(CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok())
        .is_some_and(|length| length > MAX_BACKEND_RESPONSE_BYTES as u64)
    {
        return Err(CliError::new(format!(
            "{path} response exceeds Petri's {}-byte limit",
            MAX_BACKEND_RESPONSE_BYTES
        )));
    }

    let mut bytes = Vec::with_capacity(16 * 1024);
    response
        .by_ref()
        .take((MAX_BACKEND_RESPONSE_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|error| CliError::new(format!("failed to read {path} response: {error}")))?;
    if bytes.len() > MAX_BACKEND_RESPONSE_BYTES {
        return Err(CliError::new(format!(
            "{path} response exceeds Petri's {}-byte limit",
            MAX_BACKEND_RESPONSE_BYTES
        )));
    }
    String::from_utf8(bytes)
        .map_err(|_| CliError::new(format!("{path} response is not valid UTF-8")))
}

pub(crate) fn terminal_safe_text(raw: &str) -> String {
    raw.chars()
        .take(MAX_ERROR_DISPLAY_CHARS)
        .map(|character| {
            if character.is_control() || is_bidi_control(character) {
                '\u{fffd}'
            } else {
                character
            }
        })
        .collect()
}

fn is_bidi_control(character: char) -> bool {
    matches!(
        character,
        // Unicode format controls (General_Category=Cf) are invisible or can
        // alter neighboring text. Replace the complete Unicode 15.1 set rather
        // than maintaining only the bidi subset.
        '\u{00ad}'
            | '\u{0600}'..='\u{0605}'
            | '\u{061c}'
            | '\u{06dd}'
            | '\u{070f}'
            | '\u{0890}'..='\u{0891}'
            | '\u{08e2}'
            | '\u{180e}'
            | '\u{200b}'..='\u{200f}'
            | '\u{2028}'
            | '\u{2029}'
            | '\u{202a}'..='\u{202e}'
            | '\u{2060}'..='\u{2064}'
            | '\u{2066}'..='\u{206f}'
            | '\u{feff}'
            | '\u{fff9}'..='\u{fffb}'
            | '\u{110bd}'
            | '\u{110cd}'
            | '\u{13430}'..='\u{1343f}'
            | '\u{1bca0}'..='\u{1bca3}'
            | '\u{1d173}'..='\u{1d17a}'
            | '\u{e0001}'
            | '\u{e0020}'..='\u{e007f}'
    )
}

fn parse_response_body(body: &str) -> Value {
    serde_json::from_str::<Value>(body).unwrap_or_else(|_| {
        let mut object = Map::new();
        object.insert("raw".to_string(), Value::String(body.to_string()));
        Value::Object(object)
    })
}

fn current_state_unavailable(status: StatusCode, headers_are_exact: bool, parsed: &Value) -> bool {
    let Some(object) = parsed.as_object() else {
        return false;
    };
    status == StatusCode::SERVICE_UNAVAILABLE
        && headers_are_exact
        && object.len() == 3
        && object.get("ok") == Some(&Value::Bool(false))
        && object.get("message").and_then(Value::as_str)
            == Some("current Market discovery is not authoritative")
        && object.get("code").and_then(Value::as_str) == Some("CURRENT_STATE_UNAVAILABLE")
}

fn decode_response_parts(status: StatusCode, parsed: Value, path: &str) -> Result<Value, CliError> {
    if !status.is_success() {
        let message = parsed
            .get("message")
            .and_then(Value::as_str)
            .map(str::to_string)
            .or_else(|| {
                parsed
                    .get("error")
                    .and_then(Value::as_str)
                    .map(str::to_string)
            })
            .unwrap_or_else(|| format!("backend returned HTTP {status}"));
        let code = parsed
            .get("code")
            .and_then(Value::as_str)
            .filter(|c| {
                c.len() <= 96
                    && c.bytes()
                        .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_')
            })
            .unwrap_or("AMOEBA_REQUEST_FAILED");
        let category = if status.is_server_error() {
            "unavailable"
        } else if matches!(status.as_u16(), 401 | 403 | 409) {
            "denied"
        } else {
            "validation"
        };
        return Err(CliError::coded(
            code,
            category,
            format!("{message} ({path})"),
            status.is_server_error(),
        ));
    }

    Ok(parsed)
}

pub fn json_string(value: &Value) -> Result<String, CliError> {
    let serialized = serde_json::to_string_pretty(value)
        .map_err(|error| CliError::new(format!("failed to format JSON output: {error}")))?;
    let mut escaped = String::with_capacity(serialized.len());
    for character in serialized.chars() {
        if character >= '\u{007f}' && (character.is_control() || is_bidi_control(character)) {
            write_json_unicode_escape(&mut escaped, character);
        } else {
            escaped.push(character);
        }
        if escaped.len() > MAX_TERMINAL_JSON_BYTES {
            return Err(CliError::new(format!(
                "JSON output exceeds Petri's {MAX_TERMINAL_JSON_BYTES}-byte terminal limit"
            )));
        }
    }
    Ok(escaped)
}

fn write_json_unicode_escape(output: &mut String, character: char) {
    let codepoint = character as u32;
    if codepoint <= 0xffff {
        write!(output, "\\u{codepoint:04x}").expect("writing to a String cannot fail");
        return;
    }

    let supplementary = codepoint - 0x1_0000;
    let high_surrogate = 0xd800 + (supplementary >> 10);
    let low_surrogate = 0xdc00 + (supplementary & 0x3ff);
    write!(output, "\\u{high_surrogate:04x}\\u{low_surrogate:04x}")
        .expect("writing to a String cannot fail");
}

pub fn value_at_key<'a>(value: &'a Value, keys: &[&str]) -> Option<&'a Value> {
    for key in keys {
        if let Some(found) = value.get(*key) {
            return Some(found);
        }
    }
    None
}

pub fn string_at_key(value: &Value, keys: &[&str]) -> Option<String> {
    value_at_key(value, keys).and_then(|item| match item {
        Value::String(text) => Some(text.clone()),
        Value::Number(number) => Some(number.to_string()),
        Value::Bool(flag) => Some(flag.to_string()),
        _ => None,
    })
}

pub fn array_at_key<'a>(value: &'a Value, keys: &[&str]) -> Option<&'a Vec<Value>> {
    value_at_key(value, keys).and_then(Value::as_array)
}
