from __future__ import annotations

import os
from pathlib import Path
import socket
import subprocess
import sys
import time
from contextlib import asynccontextmanager
from datetime import timedelta
from collections.abc import AsyncIterator
from urllib.request import Request, urlopen

import anyio
try:
    import httpx
except ModuleNotFoundError as error:
    if error.name != "httpx":
        raise
    import httpx2 as httpx
from mcp import ClientSession, StdioServerParameters
from mcp.client.stdio import stdio_client
from pydantic import AnyUrl

try:
    from mcp.client.streamable_http import streamable_http_client

    MODERN_STREAMABLE_HTTP = True
except ImportError:
    from mcp.client.streamable_http import streamablehttp_client as streamable_http_client

    MODERN_STREAMABLE_HTTP = False


HTTP_TOKEN = "portmate-mcp-python-http-client-check"
SDK_VERSION = os.environ.get("PORTMATE_MCP_PYTHON_SDK_VERSION", "unknown")
EXPECTED_PROTOCOL_VERSION = os.environ.get("PORTMATE_MCP_EXPECTED_PROTOCOL_VERSION", "2025-06-18")
SDK_V2 = SDK_VERSION.split(".", 1)[0] == "2"


def require(condition: bool, message: str) -> None:
    if not condition:
        raise RuntimeError(message)


def sdk_field(value: object, legacy_name: str, modern_name: str) -> object:
    if hasattr(value, modern_name):
        return getattr(value, modern_name)
    return getattr(value, legacy_name)


def bridge_binary() -> Path:
    configured = os.environ.get("PORTMATE_MCP_BINARY", "").strip()
    if configured:
        return Path(configured).resolve()
    name = "portmate-mcp.exe" if os.name == "nt" else "portmate-mcp"
    return (Path.cwd() / "target" / "debug" / name).resolve()


async def exercise_session(session: ClientSession, transport: str) -> int:
    initialized = await session.initialize()
    require(
        sdk_field(initialized, "protocolVersion", "protocol_version") == EXPECTED_PROTOCOL_VERSION,
        f"{transport} negotiated {sdk_field(initialized, 'protocolVersion', 'protocol_version')}; expected {EXPECTED_PROTOCOL_VERSION}",
    )
    server_info = sdk_field(initialized, "serverInfo", "server_info")
    require(server_info.name == "portmate-mcp", f"{transport} initialized the wrong server")

    await session.send_ping()
    tools = await session.list_tools()
    tool_names = {tool.name for tool in tools.tools}
    for tool_name in (
        "list_sessions",
        "list_transfers",
        "get_transfer",
        "start_transfer",
        "send_bytes",
        "begin_content_upload",
        "append_content_upload",
        "cancel_transfer",
        "retry_transfer",
        "create_tunnel",
        "list_tunnels",
        "stop_tunnel",
        "create_host_route",
        "list_host_routes",
        "stop_host_route",
    ):
        require(tool_name in tool_names, f"{transport} tools/list omitted {tool_name}")
    start_transfer = next((tool for tool in tools.tools if tool.name == "start_transfer"), None)
    require(start_transfer is not None, f"{transport} tools/list omitted start_transfer definition")
    start_transfer_schema = sdk_field(start_transfer, "inputSchema", "input_schema")
    protocol_schema = start_transfer_schema.get("properties", {}).get("protocol", {})
    require("tftp" in protocol_schema.get("enum", []), f"{transport} start_transfer schema omitted TFTP")
    require(len(start_transfer_schema.get("oneOf", [])) == 3,
            f"{transport} start_transfer schema did not unify all source modes")
    send_bytes = next((tool for tool in tools.tools if tool.name == "send_bytes"), None)
    require(send_bytes is not None, f"{transport} tools/list omitted send_bytes definition")
    send_bytes_schema = sdk_field(send_bytes, "inputSchema", "input_schema")
    encoding_schema = send_bytes_schema.get("properties", {}).get("encoding", {})
    require(set(("base64", "hex")).issubset(set(encoding_schema.get("enum", []))),
            f"{transport} send_bytes schema omitted a binary encoding")
    create_tunnel = next((tool for tool in tools.tools if tool.name == "create_tunnel"), None)
    require(create_tunnel is not None, f"{transport} tools/list omitted create_tunnel definition")
    create_tunnel_schema = sdk_field(create_tunnel, "inputSchema", "input_schema")
    egress_schema = create_tunnel_schema.get("properties", {}).get("egress", {})
    require("portmate-host" in egress_schema.get("enum", []), f"{transport} create_tunnel schema omitted PortMate-host egress")
    require("sessionId" not in create_tunnel_schema.get("required", []), f"{transport} create_tunnel still requires sessionId for every route")
    resources = await session.list_resources()
    require(any(str(resource.uri) == "portmate://sessions" for resource in resources.resources), f"{transport} resources/list omitted sessions")
    templates = await session.list_resource_templates()
    require(
        any(
            str(sdk_field(template, "uriTemplate", "uri_template")).startswith("portmate://sessions/{id}/")
            for template in sdk_field(templates, "resourceTemplates", "resource_templates")
        ),
        f"{transport} resources/templates/list omitted session templates",
    )
    prompts = await session.list_prompts()
    require(prompts.prompts, f"{transport} prompts/list returned no prompts")
    resource_uri = "portmate://sessions" if SDK_V2 else AnyUrl("portmate://sessions")
    sessions = await session.read_resource(resource_uri)
    require(
        sdk_field(sessions.contents[0], "mimeType", "mime_type") == "application/json",
        f"{transport} returned the wrong sessions MIME type",
    )
    return 8


async def check_stdio(binary: Path) -> None:
    environment = {
        **os.environ,
        "PORTMATE_MCP_HTTP": "0",
        "PORTMATE_MCP_CLIENT_ID": "official-python-sdk-stdio-check",
        "PORTMATE_STORE_PATH": "",
    }
    server = StdioServerParameters(command=str(binary), cwd=Path.cwd(), env=environment)
    async with stdio_client(server) as (read_stream, write_stream):
        async with ClientSession(
            read_stream,
            write_stream,
            read_timeout_seconds=10.0 if SDK_V2 else timedelta(seconds=10),
        ) as session:
            messages = await exercise_session(session, "stdio")
    print(f"MCP Python SDK {SDK_VERSION} stdio check passed ({messages} messages)")


def reserve_port() -> int:
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as listener:
        listener.bind(("127.0.0.1", 0))
        return int(listener.getsockname()[1])


def wait_for_http(endpoint: str, server: subprocess.Popen[str]) -> None:
    for _ in range(120):
        if server.poll() is not None:
            stdout, stderr = server.communicate()
            raise RuntimeError(f"PortMate HTTP bridge exited during startup\n{stdout}\n{stderr}")
        try:
            with urlopen(Request(endpoint, method="OPTIONS"), timeout=0.2) as response:
                if response.status == 204:
                    return
        except Exception:
            time.sleep(0.05)
    raise RuntimeError(f"timed out waiting for {endpoint}")


async def check_http(binary: Path) -> None:
    port = reserve_port()
    endpoint = f"http://127.0.0.1:{port}/mcp"
    environment = {
        **os.environ,
        "PORTMATE_MCP_HTTP_ADDR": f"127.0.0.1:{port}",
        "PORTMATE_MCP_HTTP_TOKEN": HTTP_TOKEN,
        "PORTMATE_MCP_CLIENT_ID": "official-python-sdk-http-check",
        "PORTMATE_STORE_PATH": "",
    }
    server = subprocess.Popen(
        [str(binary), "--http"],
        env=environment,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
    )
    try:
        wait_for_http(endpoint, server)
        async with open_http_streams(endpoint) as streams:
            read_stream, write_stream = streams[:2]
            session_id = streams[2] if len(streams) > 2 else lambda: None
            async with ClientSession(
                read_stream,
                write_stream,
                read_timeout_seconds=10.0 if SDK_V2 else timedelta(seconds=10),
            ) as session:
                requests = await exercise_session(session, "HTTP")
                require(session_id() is None, "PortMate stateless HTTP unexpectedly created a session")
        print(f"MCP Python SDK {SDK_VERSION} HTTP check passed ({requests} requests)")
    finally:
        server.terminate()
        try:
            stdout, stderr = server.communicate(timeout=2)
        except subprocess.TimeoutExpired:
            server.kill()
            stdout, stderr = server.communicate()
        if server.returncode not in (0, -15):
            print(f"PortMate HTTP bridge exited with {server.returncode}\n{stdout}\n{stderr}", file=sys.stderr)


@asynccontextmanager
async def open_http_streams(endpoint: str) -> AsyncIterator[tuple]:
    headers = {"Authorization": f"Bearer {HTTP_TOKEN}"}
    if MODERN_STREAMABLE_HTTP:
        async with httpx.AsyncClient(headers=headers, timeout=10) as http_client:
            async with streamable_http_client(endpoint, http_client=http_client) as streams:
                yield streams
    else:
        async with streamable_http_client(
            endpoint,
            headers=headers,
            timeout=10,
            sse_read_timeout=10,
        ) as streams:
            yield streams


async def main() -> None:
    binary = bridge_binary()
    require(binary.is_file(), f"PortMate MCP bridge does not exist: {binary}")
    await check_stdio(binary)
    await check_http(binary)


if __name__ == "__main__":
    anyio.run(main)
