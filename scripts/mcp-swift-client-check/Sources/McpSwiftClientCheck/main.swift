import Foundation
import MCP

#if canImport(FoundationNetworking)
    import FoundationNetworking
#endif

#if canImport(Darwin) || canImport(Glibc) || canImport(Musl)
    @preconcurrency import SystemPackage
#endif

@main
struct McpSwiftClientCheck {
    static func main() async throws {
        let arguments = CommandLine.arguments
        guard arguments.count >= 5 else {
            throw CheckError.failure(
                "Expected a mode, bridge path or endpoint, SDK version, protocol version, and optional token"
            )
        }

        let mode = arguments[1]
        let target = arguments[2]
        let sdkVersion = arguments[3]
        let protocolVersion = arguments[4]
        switch mode {
        case "stdio":
            #if canImport(Darwin) || canImport(Glibc) || canImport(Musl)
                try await checkStdio(
                    binary: target,
                    sdkVersion: sdkVersion,
                    protocolVersion: protocolVersion
                )
            #else
                throw CheckError.failure("The official Swift SDK does not provide StdioTransport on this platform")
            #endif
        case "http":
            guard arguments.count == 6 else {
                throw CheckError.failure("HTTP mode requires a bearer token")
            }
            try await checkHTTP(
                endpoint: target,
                token: arguments[5],
                sdkVersion: sdkVersion,
                protocolVersion: protocolVersion
            )
        default:
            throw CheckError.failure("Unsupported mode: \(mode)")
        }
    }

    #if canImport(Darwin) || canImport(Glibc) || canImport(Musl)
        private static func checkStdio(
            binary: String,
            sdkVersion: String,
            protocolVersion: String
        ) async throws {
            let executable = URL(fileURLWithPath: binary).standardizedFileURL
            try require(
                FileManager.default.isExecutableFile(atPath: executable.path),
                "PortMate MCP bridge is not executable: \(executable.path)"
            )

            let requestPipe = Pipe()
            let responsePipe = Pipe()
            let process = Process()
            process.executableURL = executable
            process.standardInput = requestPipe
            process.standardOutput = responsePipe
            process.standardError = FileHandle.standardError
            var environment = ProcessInfo.processInfo.environment
            environment["PORTMATE_MCP_HTTP"] = "0"
            environment["PORTMATE_MCP_CLIENT_ID"] = "official-swift-sdk-stdio-check"
            environment["PORTMATE_STORE_PATH"] = ""
            process.environment = environment
            try process.run()

            let transport = StdioTransport(
                input: FileDescriptor(rawValue: responsePipe.fileHandleForReading.fileDescriptor),
                output: FileDescriptor(rawValue: requestPipe.fileHandleForWriting.fileDescriptor)
            )
            let client = Client(name: "portmate-swift-sdk-check", version: sdkVersion)
            do {
                let initialized = try await client.connect(transport: transport)
                let messages = try await exercise(
                    client: client,
                    initialized: initialized,
                    transport: "stdio",
                    protocolVersion: protocolVersion
                )
                await client.disconnect()
                try requestPipe.fileHandleForWriting.close()
                try await waitForExit(process)
                try require(
                    process.terminationStatus == 0,
                    "PortMate stdio bridge exited with status \(process.terminationStatus)"
                )
                print("MCP Swift SDK \(sdkVersion) stdio check passed (\(messages) messages)")
            } catch {
                await client.disconnect()
                try? requestPipe.fileHandleForWriting.close()
                if process.isRunning {
                    process.terminate()
                }
                throw error
            }
        }

        private static func waitForExit(_ process: Process) async throws {
            for _ in 0..<100 where process.isRunning {
                try await Task.sleep(for: .milliseconds(20))
            }
            if process.isRunning {
                process.terminate()
                throw CheckError.failure("PortMate stdio bridge did not exit after its input closed")
            }
            process.waitUntilExit()
        }
    #endif

    private static func checkHTTP(
        endpoint: String,
        token: String,
        sdkVersion: String,
        protocolVersion: String
    ) async throws {
        guard let url = URL(string: endpoint) else {
            throw CheckError.failure("Invalid HTTP endpoint: \(endpoint)")
        }
        let transport = HTTPClientTransport(
            endpoint: url,
            streaming: false,
            protocolVersion: protocolVersion,
            requestModifier: { request in
                var request = request
                request.setValue("Bearer \(token)", forHTTPHeaderField: "Authorization")
                return request
            }
        )
        let client = Client(name: "portmate-swift-sdk-check", version: sdkVersion)
        do {
            let initialized = try await client.connect(transport: transport)
            let requests = try await exercise(
                client: client,
                initialized: initialized,
                transport: "HTTP",
                protocolVersion: protocolVersion
            )
            let sessionID = await transport.sessionID
            try require(sessionID == nil, "PortMate stateless HTTP unexpectedly created a session")
            await client.disconnect()
            print("MCP Swift SDK \(sdkVersion) HTTP check passed (\(requests) requests)")
        } catch {
            await client.disconnect()
            throw error
        }
    }

    private static func exercise(
        client: Client,
        initialized: Initialize.Result,
        transport: String,
        protocolVersion: String
    ) async throws -> Int {
        try require(
            initialized.protocolVersion == protocolVersion,
            "\(transport) negotiated \(initialized.protocolVersion); expected \(protocolVersion)"
        )
        try require(initialized.serverInfo.name == "portmate-mcp", "\(transport) initialized the wrong server")
        try await client.ping()

        let (tools, _) = try await client.listTools()
        try require(tools.contains { $0.name == "list_sessions" }, "\(transport) tools/list omitted list_sessions")

        let (resources, _) = try await client.listResources()
        try require(
            resources.contains { $0.uri == "portmate://sessions" },
            "\(transport) resources/list omitted sessions"
        )

        let (templates, _) = try await client.listResourceTemplates()
        try require(
            templates.contains { $0.uriTemplate.hasPrefix("portmate://sessions/{id}/") },
            "\(transport) resources/templates/list omitted session templates"
        )

        let (prompts, _) = try await client.listPrompts()
        try require(!prompts.isEmpty, "\(transport) prompts/list returned no prompts")

        let contents = try await client.readResource(uri: "portmate://sessions")
        try require(
            contents.first?.mimeType == "application/json",
            "\(transport) returned the wrong sessions MIME type"
        )
        return 8
    }

    private static func require(_ condition: @autoclosure () -> Bool, _ message: String) throws {
        guard condition() else {
            throw CheckError.failure(message)
        }
    }
}

enum CheckError: LocalizedError {
    case failure(String)

    var errorDescription: String? {
        switch self {
        case .failure(let message): message
        }
    }
}
