use crate::TransferProtocol;
use serde::{de::Error as _, Deserialize, Deserializer, Serialize};
use serde_json::{Map, Value};
use std::net::Ipv4Addr;
use std::time::{Duration, Instant};

pub const DEFAULT_TFTP_PORT: u16 = 69;
pub const DEFAULT_TFTP_TIMEOUT_SECONDS: u64 = 60;
const MAX_MCP_TRANSFER_ENDPOINT_BYTES: usize = 32 * 1024;
const TFTP_OPTION_FIELDS: &[&str] = &[
    "address",
    "deviceIp",
    "serverIp",
    "bindHost",
    "bindPort",
    "timeoutSeconds",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum McpStartTransferSource {
    Source,
    Inline,
    Upload,
}

pub fn classify_mcp_start_transfer_source(
    object: &Map<String, Value>,
) -> Result<McpStartTransferSource, String> {
    let has_source = object.contains_key("source");
    let has_inline = object.contains_key("fileName") || object.contains_key("contentBase64");
    let has_upload = object.contains_key("uploadId");
    let mode = match (has_source, has_inline, has_upload) {
        (true, false, false) => McpStartTransferSource::Source,
        (false, true, false)
            if object.contains_key("fileName") && object.contains_key("contentBase64") =>
        {
            McpStartTransferSource::Inline
        }
        (false, false, true) => McpStartTransferSource::Upload,
        _ => {
            return Err(
                "start_transfer requires exactly one source: source, fileName plus contentBase64, or uploadId"
                    .to_string(),
            )
        }
    };
    let allowed: &[&str] = match mode {
        McpStartTransferSource::Source => &["sessionId", "protocol", "source", "destination"],
        McpStartTransferSource::Inline => &[
            "sessionId",
            "protocol",
            "fileName",
            "contentBase64",
            "destination",
        ],
        McpStartTransferSource::Upload => &["uploadId"],
    };
    let mut unsupported = object
        .keys()
        .filter(|key| !allowed.contains(&key.as_str()))
        .map(String::as_str)
        .collect::<Vec<_>>();
    unsupported.sort_unstable();
    if unsupported.is_empty() {
        return Ok(mode);
    }
    if let Some(field) = misplaced_mcp_tftp_destination_option(object) {
        let suffix = if mode == McpStartTransferSource::Upload {
            " Resumable uploads bind their destination in begin_content_upload; start_transfer then accepts only uploadId."
        } else {
            ""
        };
        return Err(format!(
            "start_transfer TFTP option `{field}` must be nested in a structured `destination` object or encoded in the legacy load:tftpboot query string.{suffix}"
        ));
    }
    Err(format!(
        "start_transfer contains unsupported field(s): {}",
        unsupported.join(", ")
    ))
}

pub fn misplaced_mcp_tftp_destination_option(object: &Map<String, Value>) -> Option<&'static str> {
    TFTP_OPTION_FIELDS
        .iter()
        .copied()
        .find(|field| object.contains_key(*field))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum McpTransferDestination {
    Endpoint(String),
    Structured(McpStructuredTransferDestination),
}

impl<'de> Deserialize<'de> for McpTransferDestination {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = Value::deserialize(deserializer)?;
        match value {
            Value::String(endpoint) => Ok(Self::Endpoint(endpoint)),
            Value::Object(object) => {
                if !object.contains_key("kind") {
                    return Err(D::Error::custom(
                        "structured destination requires string field `kind`",
                    ));
                }
                if object.get("kind").and_then(Value::as_str) == Some("tftpboot")
                    && !object.contains_key("deviceIp")
                {
                    return Err(D::Error::custom(
                        "structured tftpboot destination requires string field `deviceIp`",
                    ));
                }
                serde_json::from_value(Value::Object(object))
                    .map(Self::Structured)
                    .map_err(D::Error::custom)
            }
            _ => Err(D::Error::custom(
                "destination must be an endpoint string or a structured destination object",
            )),
        }
    }
}

impl McpTransferDestination {
    pub fn normalize(self, protocol: &TransferProtocol) -> Result<String, String> {
        match self {
            Self::Endpoint(endpoint) => {
                validate_endpoint_text(&endpoint)?;
                if protocol == &TransferProtocol::Tftp
                    && parse_tftp_receiver_endpoint(&endpoint)?.is_none()
                {
                    return Err(
                        "TFTP transfer destination must use load:tftpboot and specify deviceIp"
                            .to_string(),
                    );
                }
                Ok(endpoint)
            }
            Self::Structured(destination) => destination.normalize(protocol),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum McpStructuredTransferDestination {
    Tftpboot {
        device_ip: String,
        #[serde(default)]
        address: Option<String>,
        #[serde(default)]
        file_name: Option<String>,
        #[serde(default)]
        server_ip: Option<String>,
        #[serde(default)]
        bind_host: Option<String>,
        #[serde(default)]
        bind_port: Option<u16>,
        #[serde(default)]
        timeout_seconds: Option<u64>,
    },
}

impl McpStructuredTransferDestination {
    fn normalize(self, protocol: &TransferProtocol) -> Result<String, String> {
        if protocol != &TransferProtocol::Tftp {
            return Err("structured tftpboot destination requires protocol `tftp`".to_string());
        }
        match self {
            Self::Tftpboot {
                device_ip,
                address,
                file_name,
                server_ip,
                bind_host,
                bind_port,
                timeout_seconds,
            } => encode_structured_tftp_destination(
                device_ip,
                address,
                file_name,
                server_ip,
                bind_host,
                bind_port,
                timeout_seconds,
            ),
        }
    }
}

fn encode_structured_tftp_destination(
    device_ip: String,
    address: Option<String>,
    file_name: Option<String>,
    server_ip: Option<String>,
    bind_host: Option<String>,
    bind_port: Option<u16>,
    timeout_seconds: Option<u64>,
) -> Result<String, String> {
    let device_ip = parse_tftp_ipv4(&device_ip, "deviceIp", false)?;
    let address = address
        .map(|value| validate_load_address(&value))
        .transpose()?;
    if let Some(file_name) = file_name.as_deref() {
        validate_tftp_file_name(file_name)?;
    }
    let server_ip = server_ip
        .map(|value| parse_tftp_ipv4(&value, "serverIp", false))
        .transpose()?;
    let bind_host = bind_host
        .map(|value| parse_tftp_ipv4(&value, "bindHost", true))
        .transpose()?;
    if let Some(seconds) = timeout_seconds {
        validate_tftp_timeout(seconds)?;
    }

    let mut serializer = url::form_urlencoded::Serializer::new(String::new());
    if let Some(value) = address.as_deref() {
        serializer.append_pair("address", value);
    }
    if let Some(value) = file_name.as_deref() {
        serializer.append_pair("fileName", value);
    }
    serializer.append_pair("deviceIp", &device_ip.to_string());
    if let Some(value) = server_ip {
        serializer.append_pair("serverIp", &value.to_string());
    }
    if let Some(value) = bind_host {
        serializer.append_pair("bindHost", &value.to_string());
    }
    if let Some(value) = bind_port {
        serializer.append_pair("bindPort", &value.to_string());
    }
    if let Some(value) = timeout_seconds {
        serializer.append_pair("timeoutSeconds", &value.to_string());
    }
    Ok(format!("load:tftpboot?{}", serializer.finish()))
}

fn validate_endpoint_text(endpoint: &str) -> Result<(), String> {
    if endpoint.is_empty()
        || endpoint.len() > MAX_MCP_TRANSFER_ENDPOINT_BYTES
        || endpoint.contains('\0')
    {
        return Err(format!(
            "destination must be non-empty, NUL-free, and at most {MAX_MCP_TRANSFER_ENDPOINT_BYTES} bytes"
        ));
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TftpReceiverSpec {
    pub address: Option<String>,
    pub file_name: Option<String>,
    pub device_ip: Ipv4Addr,
    pub server_ip: Option<Ipv4Addr>,
    pub bind_host: Option<Ipv4Addr>,
    pub bind_port: u16,
    pub timeout: Duration,
}

impl TftpReceiverSpec {
    pub fn command_lines(
        &self,
        file_name: &str,
        server_ip: Ipv4Addr,
        server_port: u16,
    ) -> Result<String, String> {
        validate_tftp_file_name(file_name)?;
        let address = self.address.as_deref().unwrap_or("${loadaddr}");
        let mut commands = format!(
            "setenv ipaddr {}\rsetenv serverip {server_ip}\r",
            self.device_ip
        );
        if server_port == DEFAULT_TFTP_PORT {
            commands.push_str("setenv tftpdstp\r");
        } else {
            commands.push_str(&format!("setenv tftpdstp {server_port}\r"));
        }
        // Newer U-Boot builds that use the LWIP network stack do not compile
        // CONFIG_TFTP_PORT and therefore ignore the tftpdstp environment
        // variable.  Their tftpboot command accepts the explicit
        // `server-ip:port:file` form. Keep the ordinary filename form for
        // port 69 so older legacy-net builds remain compatible.
        if server_port == DEFAULT_TFTP_PORT {
            commands.push_str(&format!("tftpboot {address} {file_name}\r"));
        } else {
            commands.push_str(&format!(
                "tftpboot {address} {server_ip}:{server_port}:{file_name}\r"
            ));
        }
        Ok(commands)
    }
}

pub fn parse_tftp_receiver_endpoint(value: &str) -> Result<Option<TftpReceiverSpec>, String> {
    if !value.starts_with("load:") {
        return Ok(None);
    }
    if value.starts_with("load://") {
        return Err("load: TFTP 接收端点不能包含主机部分".to_string());
    }
    let parsed =
        url::Url::parse(value).map_err(|error| format!("load: TFTP 接收端点无效: {error}"))?;
    if parsed.scheme() != "load" || parsed.path() != "tftpboot" || parsed.fragment().is_some() {
        return Err("TFTP 传输必须使用 load:tftpboot 接收端点".to_string());
    }

    let mut address = None;
    let mut file_name = None;
    let mut device_ip = None;
    let mut server_ip = None;
    let mut bind_host = None;
    let mut bind_port = None;
    let mut timeout_seconds = None;
    for (key, value) in parsed.query_pairs() {
        match key.as_ref() {
            "address" if address.is_none() => {
                address = Some(validate_load_address(value.as_ref())?);
            }
            "fileName" if file_name.is_none() => {
                validate_tftp_file_name(value.as_ref())?;
                file_name = Some(value.into_owned());
            }
            "deviceIp" if device_ip.is_none() => {
                device_ip = Some(parse_tftp_ipv4(value.as_ref(), "deviceIp", false)?);
            }
            "serverIp" if server_ip.is_none() => {
                server_ip = Some(parse_tftp_ipv4(value.as_ref(), "serverIp", false)?);
            }
            "bindHost" if bind_host.is_none() => {
                bind_host = Some(parse_tftp_ipv4(value.as_ref(), "bindHost", true)?);
            }
            "bindPort" if bind_port.is_none() => {
                bind_port = Some(
                    value
                        .parse::<u16>()
                        .map_err(|_| "load: TFTP bindPort 必须是 0 到 65535 的整数".to_string())?,
                );
            }
            "timeoutSeconds" if timeout_seconds.is_none() => {
                let seconds = value
                    .parse::<u64>()
                    .map_err(|_| "load: TFTP timeoutSeconds 必须是有效的正整数".to_string())?;
                validate_tftp_timeout(seconds)?;
                timeout_seconds = Some(seconds);
            }
            "address" | "fileName" | "deviceIp" | "serverIp" | "bindHost" | "bindPort"
            | "timeoutSeconds" => {
                return Err(format!("load: TFTP 参数 `{key}` 不能重复"));
            }
            _ => return Err(format!("load: TFTP 不支持参数 `{key}`")),
        }
    }
    let device_ip = device_ip.ok_or_else(|| "load: TFTP 必须指定 deviceIp".to_string())?;
    Ok(Some(TftpReceiverSpec {
        address,
        file_name,
        device_ip,
        server_ip,
        bind_host,
        bind_port: bind_port.unwrap_or(DEFAULT_TFTP_PORT),
        timeout: Duration::from_secs(timeout_seconds.unwrap_or(DEFAULT_TFTP_TIMEOUT_SECONDS)),
    }))
}

fn validate_load_address(value: &str) -> Result<String, String> {
    let digits = value
        .strip_prefix("0x")
        .or_else(|| value.strip_prefix("0X"))
        .unwrap_or(value);
    if digits.is_empty()
        || digits.len() > 16
        || !digits.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err("load: 加载地址必须是最多 16 位的十六进制数，可带 0x 前缀".to_string());
    }
    Ok(value.to_string())
}

fn parse_tftp_ipv4(value: &str, name: &str, allow_unspecified: bool) -> Result<Ipv4Addr, String> {
    let address = value
        .parse::<Ipv4Addr>()
        .map_err(|_| format!("load: TFTP {name} 必须是 IPv4 地址"))?;
    if address.is_multicast()
        || address == Ipv4Addr::BROADCAST
        || (!allow_unspecified && address.is_unspecified())
    {
        return Err(format!("load: TFTP {name} 不是可用的单播 IPv4 地址"));
    }
    Ok(address)
}

fn validate_tftp_timeout(seconds: u64) -> Result<(), String> {
    if seconds < 5 {
        return Err("load: TFTP timeoutSeconds 必须至少为 5".to_string());
    }
    if Instant::now()
        .checked_add(Duration::from_secs(seconds))
        .is_none()
    {
        return Err("load: TFTP timeoutSeconds 超出当前平台可表示的时间范围".to_string());
    }
    Ok(())
}

pub fn validate_tftp_file_name(file_name: &str) -> Result<(), String> {
    if file_name.is_empty()
        || file_name.len() > 255
        || file_name.starts_with('/')
        || file_name
            .split('/')
            .any(|component| component.is_empty() || matches!(component, "." | ".."))
        || !file_name.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-' | b'+' | b'/')
        })
    {
        return Err(
            "TFTP fileName 仅支持安全的相对 ASCII 路径（字母、数字、/、点、下划线、加号或连字符），且不能包含空、. 或 .. 分量"
                .to_string(),
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn structured_tftp_destination_normalizes_and_validates() {
        let destination: McpTransferDestination = serde_json::from_value(json!({
            "kind": "tftpboot",
            "deviceIp": "192.168.255.1",
            "address": "0x81800000",
            "fileName": "images/firmware.bin",
            "serverIp": "192.168.255.2",
            "bindHost": "0.0.0.0",
            "bindPort": 0,
            "timeoutSeconds": 3600
        }))
        .unwrap();
        assert_eq!(
            destination.normalize(&TransferProtocol::Tftp).unwrap(),
            "load:tftpboot?address=0x81800000&fileName=images%2Ffirmware.bin&deviceIp=192.168.255.1&serverIp=192.168.255.2&bindHost=0.0.0.0&bindPort=0&timeoutSeconds=3600"
        );

        let missing_device = serde_json::from_value::<McpTransferDestination>(json!({
            "kind": "tftpboot"
        }))
        .unwrap_err()
        .to_string();
        assert!(missing_device.contains("deviceIp"), "{missing_device}");
        let wrong_protocol: McpTransferDestination = serde_json::from_value(json!({
            "kind": "tftpboot",
            "deviceIp": "192.168.255.1"
        }))
        .unwrap();
        assert!(wrong_protocol.normalize(&TransferProtocol::Xmodem).is_err());
    }

    #[test]
    fn source_classifier_reports_misplaced_tftp_options() {
        let arguments = json!({
            "uploadId": "8d23c9bd-4d7f-45dc-86a5-c702e5ac2bce",
            "deviceIp": "192.168.255.1"
        });
        let error = classify_mcp_start_transfer_source(arguments.as_object().unwrap()).unwrap_err();
        assert!(error.contains("structured `destination`"));
        assert!(error.contains("begin_content_upload"));
        assert!(!error.contains("another source mode"));
    }
}
