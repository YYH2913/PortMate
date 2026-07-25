use std::{
    env,
    net::{IpAddr, Ipv4Addr, SocketAddr},
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};

use anyhow::{Context, Result, bail};
use rmcp::{
    ServiceExt,
    model::{
        ClientInfo, ClientRequest, Implementation, PingRequest, ProtocolVersion,
        ReadResourceRequestParams, ResourceContents, ServerResult,
    },
    transport::{
        ConfigureCommandExt, StreamableHttpClientTransport, TokioChildProcess,
        streamable_http_client::StreamableHttpClientTransportConfig,
    },
};
use tokio::{net::TcpListener, process::Command, time::timeout};

const SDK_VERSION: &str = "1.1.0";
const HTTP_TOKEN: &str = "portmate-mcp-rust-http-client-check";
const EXPECTED_PROTOCOL: ProtocolVersion = ProtocolVersion::V_2025_06_18;
const STDIO_CLIENT_ID: &str = "official-rust-sdk-stdio-check";
const HTTP_CLIENT_ID: &str = "official-rust-sdk-http-check";

#[tokio::main]
async fn main() -> Result<()> {
    let binary = binary_argument()?;

    timeout(Duration::from_secs(20), check_stdio(&binary))
        .await
        .context("stdio check exceeded 20 seconds")??;
    println!("MCP Rust SDK {SDK_VERSION} stdio check passed (8 messages)");

    timeout(Duration::from_secs(30), check_http(&binary))
        .await
        .context("HTTP check exceeded 30 seconds")??;
    println!("MCP Rust SDK {SDK_VERSION} HTTP check passed (8 requests)");
    Ok(())
}

fn binary_argument() -> Result<PathBuf> {
    let mut args = env::args_os().skip(1);
    let mut binary = None;
    while let Some(argument) = args.next() {
        if argument == "--binary" {
            let value = args.next().context("--binary requires a path")?;
            if binary.replace(PathBuf::from(value)).is_some() {
                bail!("--binary may only be provided once");
            }
        } else {
            bail!("unsupported argument: {}", argument.to_string_lossy());
        }
    }
    let binary = binary
        .or_else(|| env::var_os("PORTMATE_MCP_BINARY").map(PathBuf::from))
        .context("pass --binary or set PORTMATE_MCP_BINARY")?;
    let binary = binary
        .canonicalize()
        .with_context(|| format!("MCP bridge does not exist: {}", binary.display()))?;
    if !binary.is_file() {
        bail!("MCP bridge is not a regular file: {}", binary.display());
    }
    Ok(binary)
}

async fn check_stdio(binary: &Path) -> Result<()> {
    let transport = TokioChildProcess::new(Command::new(binary).configure(|command| {
        command
            .env("PORTMATE_MCP_HTTP", "0")
            .env("PORTMATE_STORE_PATH", "")
            .env("PORTMATE_MCP_CLIENT_ID", STDIO_CLIENT_ID)
            .stderr(Stdio::null());
    }))
    .context("spawn stdio bridge")?;
    let client = client_info()
        .serve(transport)
        .await
        .context("initialize stdio client")?;
    exercise(&client, "stdio").await?;
    client.cancel().await.context("close stdio client")?;
    Ok(())
}

async fn check_http(binary: &Path) -> Result<()> {
    let listener = TcpListener::bind(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0))
        .await
        .context("reserve HTTP port")?;
    let port = listener.local_addr()?.port();
    drop(listener);

    let endpoint = format!("http://127.0.0.1:{port}/mcp");
    let mut command = Command::new(binary);
    command
        .arg("--http")
        .env("PORTMATE_MCP_HTTP_ADDR", format!("127.0.0.1:{port}"))
        .env("PORTMATE_MCP_HTTP_TOKEN", HTTP_TOKEN)
        .env("PORTMATE_MCP_CLIENT_ID", HTTP_CLIENT_ID)
        .env("PORTMATE_STORE_PATH", "")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    let mut bridge = command.spawn().context("spawn HTTP bridge")?;

    let result = async {
        wait_for_http(port).await?;
        let config =
            StreamableHttpClientTransportConfig::with_uri(endpoint).auth_header(HTTP_TOKEN);
        let transport = StreamableHttpClientTransport::from_config(config);
        let client = client_info()
            .serve(transport)
            .await
            .context("initialize HTTP client")?;
        exercise(&client, "HTTP").await?;
        client.cancel().await.context("close HTTP client")?;
        Ok(())
    }
    .await;

    let _ = bridge.kill().await;
    let _ = bridge.wait().await;
    result
}

fn client_info() -> ClientInfo {
    ClientInfo::new(
        Default::default(),
        Implementation::new("portmate-rust-sdk-check", SDK_VERSION),
    )
}

async fn exercise<S>(
    client: &rmcp::service::RunningService<rmcp::RoleClient, S>,
    transport: &str,
) -> Result<()>
where
    S: rmcp::Service<rmcp::RoleClient>,
{
    let initialized = client
        .peer_info()
        .with_context(|| format!("{transport} client omitted initialize result"))?;
    if initialized.protocol_version != EXPECTED_PROTOCOL {
        bail!(
            "{transport} negotiated {}; expected {}",
            initialized.protocol_version,
            EXPECTED_PROTOCOL
        );
    }
    if initialized.server_info.name != "portmate-mcp" {
        bail!("{transport} initialized the wrong server");
    }

    match client
        .send_request(ClientRequest::PingRequest(PingRequest::default()))
        .await
        .with_context(|| format!("{transport} ping"))?
    {
        ServerResult::EmptyResult(_) => {}
        response => bail!("{transport} ping returned an unexpected response: {response:?}"),
    }

    let tools = client
        .list_all_tools()
        .await
        .with_context(|| format!("{transport} tools/list"))?;
    if !tools.iter().any(|tool| tool.name == "list_sessions") {
        bail!("{transport} tools/list omitted list_sessions");
    }

    let resources = client
        .list_all_resources()
        .await
        .with_context(|| format!("{transport} resources/list"))?;
    if !resources
        .iter()
        .any(|resource| resource.uri == "portmate://sessions")
    {
        bail!("{transport} resources/list omitted sessions");
    }

    let templates = client
        .list_all_resource_templates()
        .await
        .with_context(|| format!("{transport} resources/templates/list"))?;
    if !templates.iter().any(|template| {
        template
            .uri_template
            .starts_with("portmate://sessions/{id}/")
    }) {
        bail!("{transport} resources/templates/list omitted session templates");
    }

    let prompts = client
        .list_all_prompts()
        .await
        .with_context(|| format!("{transport} prompts/list"))?;
    if prompts.is_empty() {
        bail!("{transport} prompts/list returned no prompts");
    }

    let contents = client
        .read_resource(ReadResourceRequestParams::new("portmate://sessions"))
        .await
        .with_context(|| format!("{transport} resources/read"))?;
    if !contents.contents.iter().any(|content| {
        matches!(
            content,
            ResourceContents::TextResourceContents {
                mime_type: Some(mime_type),
                ..
            } if mime_type == "application/json"
        )
    }) {
        bail!("{transport} returned the wrong sessions MIME type");
    }
    Ok(())
}

async fn wait_for_http(port: u16) -> Result<()> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        match tokio::net::TcpStream::connect((Ipv4Addr::LOCALHOST, port)).await {
            Ok(stream) => {
                drop(stream);
                return Ok(());
            }
            Err(error) if tokio::time::Instant::now() < deadline => {
                tokio::time::sleep(Duration::from_millis(50)).await;
                if tokio::time::Instant::now() >= deadline {
                    return Err(error).context("timed out waiting for HTTP bridge");
                }
            }
            Err(error) => return Err(error).context("timed out waiting for HTTP bridge"),
        }
    }
}
