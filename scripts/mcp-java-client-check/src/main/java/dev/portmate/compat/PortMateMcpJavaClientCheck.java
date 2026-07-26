package dev.portmate.compat;

import io.modelcontextprotocol.client.McpClient;
import io.modelcontextprotocol.client.McpSyncClient;
import io.modelcontextprotocol.client.transport.HttpClientStreamableHttpTransport;
import io.modelcontextprotocol.client.transport.ServerParameters;
import io.modelcontextprotocol.client.transport.StdioClientTransport;
import io.modelcontextprotocol.json.McpJsonDefaults;
import io.modelcontextprotocol.spec.McpClientTransport;
import io.modelcontextprotocol.spec.McpSchema;
import java.io.IOException;
import java.net.ServerSocket;
import java.net.URI;
import java.net.http.HttpClient;
import java.net.http.HttpRequest;
import java.net.http.HttpResponse;
import java.nio.file.Files;
import java.nio.file.Path;
import java.time.Duration;
import java.util.List;
import java.util.Map;
import java.util.concurrent.TimeUnit;
import reactor.core.scheduler.Schedulers;

public final class PortMateMcpJavaClientCheck {
    private static final String SDK_VERSION = System.getProperty("portmate.mcp.sdk.version");
    private static final String EXPECTED_PROTOCOL = System.getProperty("portmate.mcp.protocol.version");
    private static final String HTTP_TOKEN = "portmate-mcp-java-http-client-check";

    private PortMateMcpJavaClientCheck() {}

    public static void main(String[] args) throws Exception {
        require(args.length == 1, "expected the PortMate MCP bridge path");
        Path binary = Path.of(args[0]).toAbsolutePath().normalize();
        require(Files.isRegularFile(binary), "PortMate MCP bridge does not exist: " + binary);
        require(SDK_VERSION != null && !SDK_VERSION.isBlank(), "missing Java SDK version");
        require(EXPECTED_PROTOCOL != null && !EXPECTED_PROTOCOL.isBlank(), "missing expected protocol version");

        try {
            checkStdio(binary);
            checkHttp(binary);
        }
        finally {
            Schedulers.shutdownNow();
        }
    }

    private static void checkStdio(Path binary) {
        ServerParameters parameters = ServerParameters.builder(binary.toString())
            .env(Map.of(
                "PORTMATE_MCP_HTTP", "0",
                "PORTMATE_MCP_CLIENT_ID", "official-java-sdk-stdio-check",
                "PORTMATE_STORE_PATH", ""))
            .build();
        StdioClientTransport transport = new StdioClientTransport(parameters, McpJsonDefaults.getMapper());
        int messages = exercise(transport, "stdio");
        System.out.printf("MCP Java SDK %s stdio check passed (%d messages)%n", SDK_VERSION, messages);
    }

    private static void checkHttp(Path binary) throws Exception {
        int port = reservePort();
        String baseUri = "http://127.0.0.1:" + port;
        String endpoint = baseUri + "/mcp";
        Process process = startHttpBridge(binary, port);
        try {
            waitForHttp(endpoint, process);
            HttpClientStreamableHttpTransport transport = HttpClientStreamableHttpTransport.builder(baseUri)
                .supportedProtocolVersions(List.of(EXPECTED_PROTOCOL))
                .httpRequestCustomizer((request, method, uri, body, context) ->
                    request.header("Authorization", "Bearer " + HTTP_TOKEN))
                .build();
            int requests = exercise(transport, "HTTP");
            System.out.printf("MCP Java SDK %s HTTP check passed (%d requests)%n", SDK_VERSION, requests);
        }
        finally {
            stopProcess(process);
        }
    }

    private static Process startHttpBridge(Path binary, int port) throws IOException {
        ProcessBuilder builder = new ProcessBuilder(binary.toString(), "--http")
            .redirectOutput(ProcessBuilder.Redirect.DISCARD)
            .redirectError(ProcessBuilder.Redirect.DISCARD);
        builder.environment().putAll(Map.of(
            "PORTMATE_MCP_HTTP_ADDR", "127.0.0.1:" + port,
            "PORTMATE_MCP_HTTP_TOKEN", HTTP_TOKEN,
            "PORTMATE_MCP_CLIENT_ID", "official-java-sdk-http-check",
            "PORTMATE_STORE_PATH", ""));
        return builder.start();
    }

    private static void waitForHttp(String endpoint, Process process) throws Exception {
        HttpClient client = HttpClient.newBuilder().connectTimeout(Duration.ofMillis(250)).build();
        HttpRequest request = HttpRequest.newBuilder(URI.create(endpoint))
            .timeout(Duration.ofMillis(250))
            .method("OPTIONS", HttpRequest.BodyPublishers.noBody())
            .build();
        for (int attempt = 0; attempt < 120; attempt++) {
            require(process.isAlive(), "PortMate HTTP bridge exited during startup");
            try {
                if (client.send(request, HttpResponse.BodyHandlers.discarding()).statusCode() == 204) return;
            }
            catch (IOException ignored) {
                // The loopback listener may not be ready yet.
            }
            Thread.sleep(50);
        }
        throw new IllegalStateException("timed out waiting for " + endpoint);
    }

    private static int exercise(McpClientTransport transport, String name) {
        McpSyncClient client = McpClient.sync(transport)
            .clientInfo(McpSchema.Implementation.builder("portmate-java-sdk-check", SDK_VERSION).build())
            .initializationTimeout(Duration.ofSeconds(10))
            .requestTimeout(Duration.ofSeconds(10))
            .build();
        try {
            McpSchema.InitializeResult initialized = client.initialize();
            require(EXPECTED_PROTOCOL.equals(initialized.protocolVersion()),
                name + " negotiated " + initialized.protocolVersion() + "; expected " + EXPECTED_PROTOCOL);
            require("portmate-mcp".equals(initialized.serverInfo().name()), name + " initialized the wrong server");

            require(client.ping() != null, name + " ping returned no result");
            require(client.listTools().tools().stream().anyMatch(tool -> "list_sessions".equals(tool.name())),
                name + " tools/list omitted list_sessions");
            require(client.listResources().resources().stream()
                .anyMatch(resource -> "portmate://sessions".equals(resource.uri())),
                name + " resources/list omitted sessions");
            require(client.listResourceTemplates().resourceTemplates().stream()
                .anyMatch(template -> template.uriTemplate().startsWith("portmate://sessions/{id}/")),
                name + " resources/templates/list omitted session templates");
            require(!client.listPrompts().prompts().isEmpty(), name + " prompts/list returned no prompts");
            McpSchema.ReadResourceResult resource = client.readResource(
                McpSchema.ReadResourceRequest.builder("portmate://sessions").build());
            require(!resource.contents().isEmpty()
                    && "application/json".equals(resource.contents().get(0).mimeType()),
                name + " returned the wrong sessions MIME type");
            return 8;
        }
        finally {
            require(client.closeGracefully(), name + " client did not close gracefully");
        }
    }

    private static int reservePort() throws IOException {
        try (ServerSocket socket = new ServerSocket(0, 1, java.net.InetAddress.getByName("127.0.0.1"))) {
            return socket.getLocalPort();
        }
    }

    private static void stopProcess(Process process) {
        if (process == null || !process.isAlive()) return;
        process.destroy();
        try {
            if (!process.waitFor(2, TimeUnit.SECONDS)) {
                process.destroyForcibly();
                process.waitFor(2, TimeUnit.SECONDS);
            }
        }
        catch (InterruptedException error) {
            Thread.currentThread().interrupt();
            process.destroyForcibly();
        }
    }

    private static void require(boolean condition, String message) {
        if (!condition) throw new IllegalStateException(message);
    }
}
