use super::*;
use std::net::Ipv4Addr;

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) enum AddressMode {
    #[default]
    Dhcp,
    Static,
    Relay,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields, default)]
pub(crate) struct EasyTierConfig {
    pub binary_directory: String,
    pub network_name: String,
    pub hostname: String,
    pub address_mode: AddressMode,
    pub virtual_ipv4: String,
    pub peers: Vec<String>,
    pub listeners: Vec<String>,
    pub latency_first: bool,
    pub mtu: u16,
}

impl Default for EasyTierConfig {
    fn default() -> Self {
        Self {
            binary_directory: String::new(),
            network_name: "PortMate".into(),
            hostname: String::new(),
            address_mode: AddressMode::Dhcp,
            virtual_ipv4: String::new(),
            peers: vec![],
            listeners: vec!["tcp://0.0.0.0:11010".into(), "udp://0.0.0.0:11010".into()],
            latency_first: false,
            mtu: 1360,
        }
    }
}

impl EasyTierConfig {
    pub(super) fn validate(&self) -> Result<(), EasyTierError> {
        if self.network_name.trim().is_empty() || invalid_text(&self.network_name, 128) {
            return Err(failure(
                "invalid-network",
                "Network name must contain 1–128 characters",
            ));
        }
        if invalid_text(&self.hostname, 128) || invalid_text(&self.binary_directory, 4096) {
            return Err(failure(
                "invalid-config",
                "Invalid hostname or executable directory",
            ));
        }
        if !self.binary_directory.is_empty() && !Path::new(&self.binary_directory).is_absolute() {
            return Err(failure(
                "invalid-config",
                "Executable directory must be absolute",
            ));
        }
        if self.address_mode == AddressMode::Static {
            let ip = self
                .virtual_ipv4
                .parse::<Ipv4Addr>()
                .map_err(|_| failure("invalid-address", "Expected a virtual IPv4 address"))?;
            if ip.is_unspecified() || ip.octets()[0] >= 224 || ip.is_loopback() {
                return Err(failure(
                    "invalid-address",
                    "Expected a unicast virtual IPv4 address",
                ));
            }
        }
        if !(576..=9000).contains(&self.mtu) {
            return Err(failure(
                "invalid-config",
                "MTU must be between 576 and 9000",
            ));
        }
        for (endpoints, listener) in [(&self.peers, false), (&self.listeners, true)] {
            if endpoints.len() > 32 {
                return Err(failure(
                    "invalid-endpoint",
                    "At most 32 endpoints are supported",
                ));
            }
            let mut unique = HashSet::new();
            for endpoint in endpoints {
                validate_endpoint(endpoint, listener)?;
                if !unique.insert(endpoint) {
                    return Err(failure("invalid-endpoint", "Duplicate endpoint"));
                }
            }
        }
        Ok(())
    }
}

fn invalid_text(value: &str, limit: usize) -> bool {
    value.chars().count() > limit || value.chars().any(char::is_control)
        // EasyTier expands ${NAME} variables when reading TOML, including secrets.
        || value.contains("${")
}

pub(super) fn validate_secret(secret: &str) -> Result<(), EasyTierError> {
    if secret.trim().is_empty() || invalid_text(secret, 1024) {
        Err(failure("invalid-secret", "Network secret must contain 1–1024 characters without control characters or variable expansion"))
    } else {
        Ok(())
    }
}

fn validate_endpoint(value: &str, listener: bool) -> Result<(), EasyTierError> {
    let invalid = || {
        failure(
            "invalid-endpoint",
            "Use tcp/udp/ws/wss/quic/wg://host:port endpoints",
        )
    };
    if invalid_text(value, 512) {
        return Err(invalid());
    }
    let url = url::Url::parse(value).map_err(|_| invalid())?;
    if !matches!(url.scheme(), "tcp" | "udp" | "ws" | "wss" | "quic" | "wg")
        || url.host_str().is_none()
        || url.port_or_known_default().is_none_or(|port| port == 0)
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(invalid());
    }
    if !listener && matches!(url.host_str(), Some("0.0.0.0" | "[::]" | "::")) {
        return Err(invalid());
    }
    Ok(())
}

pub(super) fn render_config(config: &EasyTierConfig, secret: &str, id: &str) -> String {
    let quote = |value: &str| serde_json::to_string(value).expect("strings are serializable");
    let mut text = format!(
        "instance_name = \"PortMate\"\ninstance_id = {}\ndhcp = {}\nlisteners = {}\n",
        quote(id),
        config.address_mode == AddressMode::Dhcp,
        serde_json::to_string(&config.listeners).expect("endpoints are serializable"),
    );
    if !config.hostname.is_empty() {
        text += &format!("hostname = {}\n", quote(&config.hostname));
    }
    if config.address_mode == AddressMode::Static {
        text += &format!("ipv4 = {}\n", quote(&config.virtual_ipv4));
    }
    text += &format!(
        "\n[network_identity]\nnetwork_name = {}\nnetwork_secret = {}\n",
        quote(&config.network_name),
        quote(secret)
    );
    for peer in &config.peers {
        text += &format!("\n[[peer]]\nuri = {}\n", quote(peer));
    }
    text += &format!("\n[flags]\nno_tun = {}\nmtu = {}\nlatency_first = {}\ndisable_upnp = true\nrelay_network_whitelist = \"\"\n",
        config.address_mode == AddressMode::Relay, config.mtu, config.latency_first);
    text
}

pub(super) fn load_config(root: &Path) -> Result<EasyTierConfig, EasyTierError> {
    let path = root.join("settings.json");
    if !path.exists() {
        return Ok(EasyTierConfig::default());
    }
    let file = fs::File::open(&path).map_err(|e| failure("config-failed", e))?;
    let mut bytes = Vec::new();
    std::io::Read::take(file, 64 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| failure("config-failed", e))?;
    if bytes.len() > 64 * 1024 {
        return Err(failure("config-failed", "Configuration exceeds 64 KiB"));
    }
    let config: EasyTierConfig =
        serde_json::from_slice(&bytes).map_err(|e| failure("config-failed", e))?;
    config.validate()?;
    Ok(config)
}

pub(super) fn save_config(root: &Path, config: &EasyTierConfig) -> Result<(), EasyTierError> {
    config.validate()?;
    private_directory(root)?;
    let mut file =
        tempfile::NamedTempFile::new_in(root).map_err(|e| failure("config-failed", e))?;
    serde_json::to_writer_pretty(&mut file, config).map_err(|e| failure("config-failed", e))?;
    file.as_file()
        .sync_all()
        .map_err(|e| failure("config-failed", e))?;
    file.persist(root.join("settings.json"))
        .map_err(|e| failure("config-failed", e.error))?;
    Ok(())
}
