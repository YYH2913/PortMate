package dev.portmate.compat

import io.ktor.client.HttpClient
import io.ktor.client.engine.cio.CIO
import io.ktor.client.plugins.sse.SSE
import io.ktor.client.request.header
import io.ktor.http.HttpHeaders
import io.modelcontextprotocol.kotlin.sdk.client.Client
import io.modelcontextprotocol.kotlin.sdk.client.StdioClientTransport
import io.modelcontextprotocol.kotlin.sdk.client.StreamableHttpClientTransport
import io.modelcontextprotocol.kotlin.sdk.types.Implementation
import io.modelcontextprotocol.kotlin.sdk.types.ListResourceTemplatesRequest
import io.modelcontextprotocol.kotlin.sdk.types.ReadResourceRequest
import io.modelcontextprotocol.kotlin.sdk.types.ReadResourceRequestParams
import io.modelcontextprotocol.kotlin.sdk.types.SUPPORTED_PROTOCOL_VERSIONS
import java.io.IOException
import java.net.InetAddress
import java.net.ServerSocket
import java.net.URI
import java.net.http.HttpRequest
import java.net.http.HttpResponse
import java.nio.file.Files
import java.nio.file.Path
import java.time.Duration
import java.util.concurrent.TimeUnit
import kotlinx.coroutines.DelicateCoroutinesApi
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import kotlinx.io.asSink
import kotlinx.io.asSource
import kotlinx.io.buffered
import java.net.http.HttpClient as JdkHttpClient

private val sdkVersion = requireNotNull(System.getProperty("portmate.mcp.sdk.version"))
private val expectedProtocol = requireNotNull(System.getProperty("portmate.mcp.protocol.version"))
private const val httpToken = "portmate-mcp-kotlin-http-client-check"

@OptIn(DelicateCoroutinesApi::class)
fun main(args: Array<String>): Unit = runBlocking {
    try {
        require(args.size == 1) { "expected the PortMate MCP bridge path" }
        val binary = Path.of(args[0]).toAbsolutePath().normalize()
        require(Files.isRegularFile(binary)) { "PortMate MCP bridge does not exist: $binary" }
        require(sdkVersion.isNotBlank()) { "missing Kotlin SDK version" }
        require(expectedProtocol.isNotBlank()) { "missing expected protocol version" }
        require(expectedProtocol in SUPPORTED_PROTOCOL_VERSIONS) {
            "Kotlin SDK $sdkVersion does not support protocol $expectedProtocol"
        }

        checkStdio(binary)
        checkHttp(binary)
    } finally {
        Dispatchers.shutdown()
    }
}

private suspend fun checkStdio(binary: Path) {
    val process = startBridge(binary, listOf(), mapOf(
        "PORTMATE_MCP_HTTP" to "0",
        "PORTMATE_MCP_CLIENT_ID" to "official-kotlin-sdk-stdio-check",
        "PORTMATE_STORE_PATH" to "",
    ))
    val client = Client(Implementation("portmate-kotlin-sdk-check", sdkVersion))
    try {
        val transport = StdioClientTransport(
            input = process.inputStream.asSource().buffered(),
            output = process.outputStream.asSink().buffered(),
            error = process.errorStream.asSource().buffered(),
            classifyStderr = { StdioClientTransport.StderrSeverity.IGNORE },
        )
        val messages = withTimeout(20_000) {
            client.connect(transport)
            exercise(client, "stdio")
        }
        println("MCP Kotlin SDK $sdkVersion stdio check passed ($messages messages)")
    } finally {
        closeClient(client)
        stopProcess(process)
    }
}

private suspend fun checkHttp(binary: Path) {
    val port = reservePort()
    val endpoint = "http://127.0.0.1:$port/mcp"
    val process = startBridge(binary, listOf("--http"), mapOf(
        "PORTMATE_MCP_HTTP_ADDR" to "127.0.0.1:$port",
        "PORTMATE_MCP_HTTP_TOKEN" to httpToken,
        "PORTMATE_MCP_CLIENT_ID" to "official-kotlin-sdk-http-check",
        "PORTMATE_STORE_PATH" to "",
    ))
    val httpClient = HttpClient(CIO) { install(SSE) }
    val transport = StreamableHttpClientTransport(
        client = httpClient,
        url = endpoint,
        requestBuilder = { header(HttpHeaders.Authorization, "Bearer $httpToken") },
    ).apply {
        protocolVersion = expectedProtocol
    }
    val client = Client(Implementation("portmate-kotlin-sdk-check", sdkVersion))
    try {
        waitForHttp(endpoint, process)
        val requests = withTimeout(30_000) {
            client.connect(transport)
            exercise(client, "HTTP")
        }
        check(transport.sessionId == null) { "PortMate stateless HTTP unexpectedly created a session" }
        println("MCP Kotlin SDK $sdkVersion HTTP check passed ($requests requests)")
    } finally {
        closeClient(client)
        httpClient.close()
        stopProcess(process)
    }
}

private suspend fun exercise(client: Client, transport: String): Int {
    check(client.serverVersion?.name == "portmate-mcp") { "$transport initialized the wrong server" }
    client.ping()
    check(client.listTools().tools.any { it.name == "list_sessions" }) {
        "$transport tools/list omitted list_sessions"
    }
    check(client.listResources().resources.any { it.uri == "portmate://sessions" }) {
        "$transport resources/list omitted sessions"
    }
    check(client.listResourceTemplates(ListResourceTemplatesRequest()).resourceTemplates.any {
        it.uriTemplate.startsWith("portmate://sessions/{id}/")
    }) { "$transport resources/templates/list omitted session templates" }
    check(client.listPrompts().prompts.isNotEmpty()) { "$transport prompts/list returned no prompts" }
    val resource = client.readResource(ReadResourceRequest(ReadResourceRequestParams("portmate://sessions")))
    check(resource.contents.firstOrNull()?.mimeType == "application/json") {
        "$transport returned the wrong sessions MIME type"
    }
    return 8
}

private fun startBridge(binary: Path, arguments: List<String>, environment: Map<String, String>): Process {
    val builder = ProcessBuilder(listOf(binary.toString()) + arguments)
    builder.environment().putAll(environment)
    return builder.start()
}

private fun reservePort(): Int = ServerSocket(0, 1, InetAddress.getByName("127.0.0.1")).use { it.localPort }

private fun waitForHttp(endpoint: String, process: Process) {
    val client = JdkHttpClient.newBuilder().connectTimeout(Duration.ofMillis(250)).build()
    val request = HttpRequest.newBuilder(URI.create(endpoint))
        .timeout(Duration.ofMillis(250))
        .method("OPTIONS", HttpRequest.BodyPublishers.noBody())
        .build()
    repeat(120) {
        check(process.isAlive) { "PortMate HTTP bridge exited during startup" }
        try {
            if (client.send(request, HttpResponse.BodyHandlers.discarding()).statusCode() == 204) return
        } catch (_: IOException) {
            // The loopback listener may not be ready yet.
        }
        Thread.sleep(50)
    }
    error("timed out waiting for $endpoint")
}

private suspend fun closeClient(client: Client) {
    withTimeout(3_000) { client.close() }
}

private fun stopProcess(process: Process) {
    if (!process.isAlive) return
    process.destroy()
    if (!process.waitFor(2, TimeUnit.SECONDS)) {
        process.destroyForcibly()
        process.waitFor(2, TimeUnit.SECONDS)
    }
}
