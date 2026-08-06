# frozen_string_literal: true

sdk_version = ENV.fetch("PORTMATE_MCP_RUBY_SDK_VERSION")
gem "mcp", sdk_version
gem "faraday", ENV.fetch("PORTMATE_MCP_RUBY_FARADAY_VERSION")
gem "event_stream_parser", ENV.fetch("PORTMATE_MCP_RUBY_EVENT_STREAM_VERSION")

%w[HTTP_PROXY HTTPS_PROXY ALL_PROXY http_proxy https_proxy all_proxy].each { |name| ENV.delete(name) }

require "mcp"
require "net/http"
require "socket"
require "timeout"
require "uri"

EXPECTED_PROTOCOL = ENV.fetch("PORTMATE_MCP_EXPECTED_PROTOCOL_VERSION")
HTTP_TOKEN = "portmate-mcp-ruby-http-client-check"

def require_condition(condition, message)
  raise message unless condition
end

def exercise(client, transport)
  initialized = client.connect(
    client_info: { name: "portmate-ruby-sdk-check", version: ENV.fetch("PORTMATE_MCP_RUBY_SDK_VERSION") },
    protocol_version: EXPECTED_PROTOCOL,
  )
  require_condition(
    initialized["protocolVersion"] == EXPECTED_PROTOCOL,
    "#{transport} negotiated #{initialized["protocolVersion"].inspect}; expected #{EXPECTED_PROTOCOL}",
  )
  require_condition(initialized.dig("serverInfo", "name") == "portmate-mcp", "#{transport} initialized the wrong server")

  require_condition(client.ping == {}, "#{transport} ping returned a non-empty result")
  require_condition(client.tools.any? { |tool| tool.name == "list_sessions" }, "#{transport} tools/list omitted list_sessions")
  require_condition(
    client.resources.any? { |resource| resource["uri"] == "portmate://sessions" },
    "#{transport} resources/list omitted sessions",
  )
  require_condition(
    client.resource_templates.any? { |template| template["uriTemplate"].start_with?("portmate://sessions/{id}/") },
    "#{transport} resources/templates/list omitted session templates",
  )
  require_condition(!client.prompts.empty?, "#{transport} prompts/list returned no prompts")
  contents = client.read_resource(uri: "portmate://sessions")
  require_condition(contents.first&.fetch("mimeType", nil) == "application/json", "#{transport} returned the wrong sessions MIME type")
  8
end

def check_stdio(binary)
  transport = MCP::Client::Stdio.new(
    command: binary,
    # A separate argument keeps Process.spawn in argv mode so paths with spaces stay intact.
    args: ["--stdio"],
    env: ENV.to_h.merge(
      "PORTMATE_MCP_HTTP" => "0",
      "PORTMATE_MCP_CLIENT_ID" => "official-ruby-sdk-stdio-check",
      "PORTMATE_STORE_PATH" => "",
    ),
    read_timeout: 10,
  )
  client = MCP::Client.new(transport: transport)
  messages = Timeout.timeout(20) { exercise(client, "stdio") }
  puts "MCP Ruby SDK #{ENV.fetch("PORTMATE_MCP_RUBY_SDK_VERSION")} stdio check passed (#{messages} messages)"
ensure
  transport&.close
end

def reserve_port
  listener = TCPServer.new("127.0.0.1", 0)
  listener.local_address.ip_port
ensure
  listener&.close
end

def wait_for_http(endpoint, pid)
  uri = URI(endpoint)
  120.times do
    Process.kill(0, pid)
    begin
      response = Net::HTTP.start(uri.host, uri.port, open_timeout: 0.2, read_timeout: 0.2) do |http|
        http.request(Net::HTTP::Options.new(uri.request_uri))
      end
      return if response.code.to_i == 204
    rescue IOError, SystemCallError, Timeout::Error
      sleep 0.05
    end
  end
  raise "timed out waiting for #{endpoint}"
rescue Errno::ESRCH
  raise "PortMate HTTP bridge exited during startup"
end

def stop_process(pid)
  return unless pid

  Process.kill(Gem.win_platform? ? "KILL" : "TERM", pid)
  Timeout.timeout(2) { Process.wait(pid) }
rescue Timeout::Error
  Process.kill("KILL", pid)
  Process.wait(pid)
rescue Errno::ESRCH, Errno::ECHILD
  nil
end

def check_http(binary)
  port = reserve_port
  endpoint = "http://127.0.0.1:#{port}/mcp"
  pid = Process.spawn(
    ENV.to_h.merge(
      "PORTMATE_MCP_HTTP_ADDR" => "127.0.0.1:#{port}",
      "PORTMATE_MCP_HTTP_TOKEN" => HTTP_TOKEN,
      "PORTMATE_MCP_CLIENT_ID" => "official-ruby-sdk-http-check",
      "PORTMATE_STORE_PATH" => "",
    ),
    binary,
    "--http",
    out: File::NULL,
    err: File::NULL,
  )
  wait_for_http(endpoint, pid)
  transport = MCP::Client::HTTP.new(
    url: endpoint,
    headers: { "Authorization" => "Bearer #{HTTP_TOKEN}" },
  )
  client = MCP::Client.new(transport: transport)
  requests = Timeout.timeout(30) { exercise(client, "HTTP") }
  require_condition(transport.session_id.nil?, "PortMate stateless HTTP unexpectedly created a session")
  puts "MCP Ruby SDK #{ENV.fetch("PORTMATE_MCP_RUBY_SDK_VERSION")} HTTP check passed (#{requests} requests)"
ensure
  transport&.close
  stop_process(pid)
end

binary = File.expand_path(ENV.fetch("PORTMATE_MCP_BINARY"))
require_condition(File.file?(binary), "PortMate MCP bridge does not exist: #{binary}")
check_stdio(binary)
check_http(binary)
