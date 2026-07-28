package main

import (
	"context"
	"errors"
	"flag"
	"fmt"
	"io"
	"net"
	"net/http"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"time"

	"github.com/modelcontextprotocol/go-sdk/mcp"
)

const (
	httpToken             = "portmate-mcp-go-http-client-check"
	stdioClientID         = "official-go-sdk-stdio-check"
	httpClientID          = "official-go-sdk-http-check"
	defaultBinaryRelative = "../../target/debug/portmate-mcp"
)

var (
	sdkVersion       = environmentValue("PORTMATE_MCP_GO_SDK_VERSION", "1.7.0")
	expectedProtocol = environmentValue("PORTMATE_MCP_EXPECTED_PROTOCOL_VERSION", "2025-06-18")
)

func main() {
	binaryFlag := flag.String("binary", "", "path to the portmate-mcp executable")
	flag.Parse()
	binary := strings.TrimSpace(*binaryFlag)
	if binary == "" {
		binary = strings.TrimSpace(os.Getenv("PORTMATE_MCP_BINARY"))
	}
	if binary == "" {
		binary = defaultBinaryRelative
		if filepath.Ext(os.Args[0]) == ".exe" {
			binary += ".exe"
		}
	}
	binary, err := filepath.Abs(binary)
	if err != nil {
		fatal(err)
	}

	if err := checkStdio(binary); err != nil {
		fatal(fmt.Errorf("stdio: %w", err))
	}
	fmt.Printf("MCP Go SDK %s stdio check passed (8 messages)\n", sdkVersion)

	if err := checkHTTP(binary); err != nil {
		fatal(fmt.Errorf("HTTP: %w", err))
	}
	fmt.Printf("MCP Go SDK %s HTTP check passed (8 requests)\n", sdkVersion)
}

func checkStdio(binary string) error {
	ctx, cancel := context.WithTimeout(context.Background(), 15*time.Second)
	defer cancel()

	cmd := exec.Command(binary)
	cmd.Env = append(os.Environ(),
		"PORTMATE_MCP_HTTP=0",
		"PORTMATE_STORE_PATH=",
		"PORTMATE_MCP_CLIENT_ID="+stdioClientID,
	)
	cmd.Stderr = io.Discard
	stdout, err := cmd.StdoutPipe()
	if err != nil {
		return err
	}
	stdin, err := cmd.StdinPipe()
	if err != nil {
		return err
	}
	if err := cmd.Start(); err != nil {
		return err
	}

	client := newClient()
	session, err := client.Connect(ctx, &mcp.IOTransport{Reader: stdout, Writer: stdin}, nil)
	if err != nil {
		_ = cmd.Process.Kill()
		_ = cmd.Wait()
		return err
	}
	exerciseErr := exercise(ctx, session, "stdio")
	closeErr := session.Close()
	waitErr := cmd.Wait()
	if exerciseErr != nil {
		return exerciseErr
	}
	if closeErr != nil && !errors.Is(closeErr, mcp.ErrConnectionClosed) {
		return fmt.Errorf("close: %w", closeErr)
	}
	if waitErr != nil {
		return fmt.Errorf("bridge exited after stdio close: %w", waitErr)
	}
	return nil
}

func checkHTTP(binary string) error {
	ctx, cancel := context.WithTimeout(context.Background(), 25*time.Second)
	defer cancel()

	port, err := reservePort()
	if err != nil {
		return err
	}
	endpoint := fmt.Sprintf("http://127.0.0.1:%d/mcp", port)
	cmd := exec.Command(binary, "--http")
	cmd.Env = append(os.Environ(),
		"PORTMATE_MCP_HTTP_ADDR=127.0.0.1:"+fmt.Sprint(port),
		"PORTMATE_MCP_HTTP_TOKEN="+httpToken,
		"PORTMATE_MCP_CLIENT_ID="+httpClientID,
		"PORTMATE_STORE_PATH=",
	)
	cmd.Stdout = io.Discard
	cmd.Stderr = io.Discard
	if err := cmd.Start(); err != nil {
		return err
	}
	defer func() {
		if cmd.Process != nil {
			_ = cmd.Process.Kill()
		}
		_ = cmd.Wait()
	}()

	if err := waitForHTTP(ctx, endpoint); err != nil {
		return err
	}
	transport := &mcp.StreamableClientTransport{
		Endpoint:             endpoint,
		HTTPClient:           &http.Client{Transport: bearerTransport{base: http.DefaultTransport, token: httpToken}, Timeout: 10 * time.Second},
		DisableStandaloneSSE: true,
		MaxRetries:           -1,
	}
	session, err := newClient().Connect(ctx, transport, nil)
	if err != nil {
		return err
	}
	exerciseErr := exercise(ctx, session, "HTTP")
	closeErr := session.Close()
	if exerciseErr != nil {
		return exerciseErr
	}
	if closeErr != nil && !errors.Is(closeErr, mcp.ErrConnectionClosed) {
		return fmt.Errorf("close: %w", closeErr)
	}
	return nil
}

func newClient() *mcp.Client {
	return mcp.NewClient(&mcp.Implementation{Name: "portmate-go-sdk-check", Version: sdkVersion}, nil)
}

func exercise(ctx context.Context, session *mcp.ClientSession, transport string) error {
	initialized := session.InitializeResult()
	if initialized == nil || initialized.ProtocolVersion != expectedProtocol {
		return fmt.Errorf("%s negotiated %v; expected %s", transport, initialized, expectedProtocol)
	}
	if initialized.ServerInfo == nil || initialized.ServerInfo.Name != "portmate-mcp" {
		return fmt.Errorf("%s initialized the wrong server", transport)
	}
	if err := session.Ping(ctx, nil); err != nil {
		return fmt.Errorf("%s ping: %w", transport, err)
	}
	tools, err := session.ListTools(ctx, nil)
	if err != nil {
		return fmt.Errorf("%s tools/list: %w", transport, err)
	}
	if !anyTool(tools.Tools, "list_sessions") {
		return fmt.Errorf("%s tools/list omitted list_sessions", transport)
	}
	resources, err := session.ListResources(ctx, nil)
	if err != nil {
		return fmt.Errorf("%s resources/list: %w", transport, err)
	}
	if !anyResource(resources.Resources, "portmate://sessions") {
		return fmt.Errorf("%s resources/list omitted sessions", transport)
	}
	templates, err := session.ListResourceTemplates(ctx, nil)
	if err != nil {
		return fmt.Errorf("%s resources/templates/list: %w", transport, err)
	}
	if !anyTemplate(templates.ResourceTemplates, "portmate://sessions/{id}/") {
		return fmt.Errorf("%s resources/templates/list omitted session templates", transport)
	}
	prompts, err := session.ListPrompts(ctx, nil)
	if err != nil {
		return fmt.Errorf("%s prompts/list: %w", transport, err)
	}
	if len(prompts.Prompts) == 0 {
		return fmt.Errorf("%s prompts/list returned no prompts", transport)
	}
	contents, err := session.ReadResource(ctx, &mcp.ReadResourceParams{URI: "portmate://sessions"})
	if err != nil {
		return fmt.Errorf("%s resources/read: %w", transport, err)
	}
	if len(contents.Contents) == 0 || contents.Contents[0].MIMEType != "application/json" {
		return fmt.Errorf("%s returned the wrong sessions MIME type", transport)
	}
	return nil
}

func anyTool(tools []*mcp.Tool, name string) bool {
	for _, tool := range tools {
		if tool != nil && tool.Name == name {
			return true
		}
	}
	return false
}

func anyResource(resources []*mcp.Resource, uri string) bool {
	for _, resource := range resources {
		if resource != nil && resource.URI == uri {
			return true
		}
	}
	return false
}

func anyTemplate(templates []*mcp.ResourceTemplate, prefix string) bool {
	for _, template := range templates {
		if template != nil && strings.HasPrefix(template.URITemplate, prefix) {
			return true
		}
	}
	return false
}

func reservePort() (int, error) {
	listener, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		return 0, err
	}
	defer listener.Close()
	return listener.Addr().(*net.TCPAddr).Port, nil
}

func waitForHTTP(ctx context.Context, endpoint string) error {
	client := &http.Client{Timeout: 500 * time.Millisecond}
	for {
		request, err := http.NewRequestWithContext(ctx, http.MethodOptions, endpoint, nil)
		if err != nil {
			return err
		}
		response, err := client.Do(request)
		if err == nil {
			response.Body.Close()
			if response.StatusCode == http.StatusNoContent {
				return nil
			}
		}
		select {
		case <-ctx.Done():
			return fmt.Errorf("timed out waiting for %s: %w", endpoint, ctx.Err())
		case <-time.After(50 * time.Millisecond):
		}
	}
}

type bearerTransport struct {
	base  http.RoundTripper
	token string
}

func (t bearerTransport) RoundTrip(request *http.Request) (*http.Response, error) {
	base := t.base
	if base == nil {
		base = http.DefaultTransport
	}
	clone := request.Clone(request.Context())
	clone.Header.Set("Authorization", "Bearer "+t.token)
	return base.RoundTrip(clone)
}

func fatal(err error) {
	fmt.Fprintln(os.Stderr, err)
	os.Exit(1)
}

func environmentValue(name, fallback string) string {
	if value := strings.TrimSpace(os.Getenv(name)); value != "" {
		return value
	}
	return fallback
}
