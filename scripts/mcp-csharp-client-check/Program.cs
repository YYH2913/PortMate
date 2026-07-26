using System.Diagnostics;
using System.Net;
using System.Net.Sockets;
using ModelContextProtocol.Client;
using ModelContextProtocol.Protocol;

if (args.Length != 3)
{
    throw new InvalidOperationException("Expected the PortMate MCP bridge path, SDK version, and protocol version");
}

string binary = Path.GetFullPath(args[0]);
string sdkVersion = args[1];
string expectedProtocol = args[2];
Require(File.Exists(binary), $"PortMate MCP bridge does not exist: {binary}");

await CheckStdioAsync(binary, sdkVersion, expectedProtocol);
await CheckHttpAsync(binary, sdkVersion, expectedProtocol);

static McpClientOptions ClientOptions(string sdkVersion, string expectedProtocol) => new()
{
    ClientInfo = new Implementation { Name = "portmate-csharp-sdk-check", Version = sdkVersion },
    ProtocolVersion = expectedProtocol,
    InitializationTimeout = TimeSpan.FromSeconds(10),
};

static async Task CheckStdioAsync(string binary, string sdkVersion, string expectedProtocol)
{
    var environment = new Dictionary<string, string?>
    {
        ["PORTMATE_MCP_HTTP"] = "0",
        ["PORTMATE_MCP_CLIENT_ID"] = "official-csharp-sdk-stdio-check",
        ["PORTMATE_STORE_PATH"] = "",
    };
    var transport = new StdioClientTransport(new StdioClientTransportOptions
    {
        Name = "PortMate MCP stdio",
        Command = binary,
        Arguments = [],
        EnvironmentVariables = environment,
    });
    using var timeout = new CancellationTokenSource(TimeSpan.FromSeconds(30));
    await using McpClient client = await McpClient.CreateAsync(
        transport,
        ClientOptions(sdkVersion, expectedProtocol),
        cancellationToken: timeout.Token);
    int messages = await ExerciseAsync(client, "stdio", expectedProtocol, timeout.Token);
    Console.WriteLine($"MCP C# SDK {sdkVersion} stdio check passed ({messages} messages)");
}

static async Task CheckHttpAsync(string binary, string sdkVersion, string expectedProtocol)
{
    int port = ReservePort();
    string endpoint = $"http://127.0.0.1:{port}/mcp";
    using Process process = StartHttpBridge(binary, port);
    try
    {
        await WaitForHttpAsync(endpoint, process);
        var transport = new HttpClientTransport(new HttpClientTransportOptions
        {
            Name = "PortMate MCP HTTP",
            Endpoint = new Uri(endpoint),
            TransportMode = HttpTransportMode.StreamableHttp,
            ConnectionTimeout = TimeSpan.FromSeconds(10),
            AdditionalHeaders = new Dictionary<string, string>
            {
                ["Authorization"] = $"Bearer {Constants.HttpToken}",
            },
        });
        using var timeout = new CancellationTokenSource(TimeSpan.FromSeconds(40));
        await using McpClient client = await McpClient.CreateAsync(
            transport,
            ClientOptions(sdkVersion, expectedProtocol),
            cancellationToken: timeout.Token);
        int requests = await ExerciseAsync(client, "HTTP", expectedProtocol, timeout.Token);
        Require(client.SessionId is null, "PortMate stateless HTTP unexpectedly created a session");
        Console.WriteLine($"MCP C# SDK {sdkVersion} HTTP check passed ({requests} requests)");
    }
    finally
    {
        StopProcess(process);
    }
}

static async Task<int> ExerciseAsync(
    McpClient client,
    string transport,
    string expectedProtocol,
    CancellationToken cancellationToken)
{
    Require(client.NegotiatedProtocolVersion == expectedProtocol,
        $"{transport} negotiated {client.NegotiatedProtocolVersion}; expected {expectedProtocol}");
    Require(client.ServerInfo.Name == "portmate-mcp", $"{transport} initialized the wrong server");
    await client.PingAsync(cancellationToken: cancellationToken);
    Require((await client.ListToolsAsync(cancellationToken: cancellationToken)).Any(tool => tool.Name == "list_sessions"),
        $"{transport} tools/list omitted list_sessions");
    Require((await client.ListResourcesAsync(cancellationToken: cancellationToken))
        .Any(resource => resource.Uri == "portmate://sessions"),
        $"{transport} resources/list omitted sessions");
    Require((await client.ListResourceTemplatesAsync(cancellationToken: cancellationToken))
        .Any(template => template.UriTemplate.StartsWith("portmate://sessions/{id}/", StringComparison.Ordinal)),
        $"{transport} resources/templates/list omitted session templates");
    Require((await client.ListPromptsAsync(cancellationToken: cancellationToken)).Count > 0,
        $"{transport} prompts/list returned no prompts");
    ReadResourceResult resource = await client.ReadResourceAsync(
        "portmate://sessions",
        cancellationToken: cancellationToken);
    Require(resource.Contents.FirstOrDefault()?.MimeType == "application/json",
        $"{transport} returned the wrong sessions MIME type");
    return 8;
}

static Process StartHttpBridge(string binary, int port)
{
    var startInfo = new ProcessStartInfo(binary, "--http")
    {
        UseShellExecute = false,
        RedirectStandardOutput = true,
        RedirectStandardError = true,
    };
    startInfo.Environment["PORTMATE_MCP_HTTP_ADDR"] = $"127.0.0.1:{port}";
    startInfo.Environment["PORTMATE_MCP_HTTP_TOKEN"] = Constants.HttpToken;
    startInfo.Environment["PORTMATE_MCP_CLIENT_ID"] = "official-csharp-sdk-http-check";
    startInfo.Environment["PORTMATE_STORE_PATH"] = "";
    Process process = Process.Start(startInfo) ?? throw new InvalidOperationException("Failed to start PortMate HTTP bridge");
    process.BeginOutputReadLine();
    process.BeginErrorReadLine();
    return process;
}

static int ReservePort()
{
    var listener = new TcpListener(IPAddress.Loopback, 0);
    listener.Start();
    try
    {
        return ((IPEndPoint)listener.LocalEndpoint).Port;
    }
    finally
    {
        listener.Stop();
    }
}

static async Task WaitForHttpAsync(string endpoint, Process process)
{
    using var client = new HttpClient { Timeout = TimeSpan.FromMilliseconds(250) };
    for (int attempt = 0; attempt < 120; attempt++)
    {
        Require(!process.HasExited, "PortMate HTTP bridge exited during startup");
        try
        {
            using var request = new HttpRequestMessage(HttpMethod.Options, endpoint);
            using HttpResponseMessage response = await client.SendAsync(request);
            if (response.StatusCode == HttpStatusCode.NoContent)
            {
                return;
            }
        }
        catch (HttpRequestException)
        {
        }
        catch (TaskCanceledException)
        {
        }
        await Task.Delay(50);
    }
    throw new TimeoutException($"Timed out waiting for {endpoint}");
}

static void StopProcess(Process process)
{
    if (process.HasExited)
    {
        return;
    }
    process.Kill(entireProcessTree: true);
    process.WaitForExit(2_000);
}

static void Require(bool condition, string message)
{
    if (!condition)
    {
        throw new InvalidOperationException(message);
    }
}

static class Constants
{
    public const string HttpToken = "portmate-mcp-csharp-http-client-check";
}
