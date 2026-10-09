package main

import (
	"crypto/ed25519"
	"crypto/rand"
	"crypto/subtle"
	"errors"
	"io"
	"log"
	"net"

	"github.com/pkg/sftp"
	"golang.org/x/crypto/ssh"
)

const (
	listenAddress = ":22"
	username      = "portmate"
	password      = "portmate"
)

func main() {
	config, err := serverConfig()
	if err != nil {
		log.Fatal(err)
	}

	listener, err := net.Listen("tcp", listenAddress)
	if err != nil {
		log.Fatal(err)
	}
	defer listener.Close()
	log.Printf("PortMate github.com/pkg/sftp compatibility server is listening on %s", listenAddress)

	for {
		connection, err := listener.Accept()
		if err != nil {
			log.Printf("accept failed: %v", err)
			continue
		}
		go serveConnection(connection, config)
	}
}

func serverConfig() (*ssh.ServerConfig, error) {
	_, privateKey, err := ed25519.GenerateKey(rand.Reader)
	if err != nil {
		return nil, err
	}
	signer, err := ssh.NewSignerFromKey(privateKey)
	if err != nil {
		return nil, err
	}

	config := &ssh.ServerConfig{
		PasswordCallback: func(metadata ssh.ConnMetadata, candidate []byte) (*ssh.Permissions, error) {
			validUser := subtle.ConstantTimeCompare([]byte(metadata.User()), []byte(username)) == 1
			validPassword := subtle.ConstantTimeCompare(candidate, []byte(password)) == 1
			if validUser && validPassword {
				return nil, nil
			}
			return nil, errors.New("password authentication rejected")
		},
	}
	config.AddHostKey(signer)
	return config, nil
}

func serveConnection(connection net.Conn, config *ssh.ServerConfig) {
	defer connection.Close()
	_, channels, requests, err := ssh.NewServerConn(connection, config)
	if err != nil {
		return
	}
	go ssh.DiscardRequests(requests)

	for channelRequest := range channels {
		if channelRequest.ChannelType() != "session" {
			_ = channelRequest.Reject(ssh.UnknownChannelType, "only session channels are supported")
			continue
		}
		channel, requests, err := channelRequest.Accept()
		if err != nil {
			continue
		}
		go serveChannel(channel, requests)
	}
}

func serveChannel(channel ssh.Channel, requests <-chan *ssh.Request) {
	defer channel.Close()
	for request := range requests {
		accepted := request.Type == "subsystem" && subsystemName(request.Payload) == "sftp"
		if request.WantReply {
			_ = request.Reply(accepted, nil)
		}
		if !accepted {
			continue
		}

		go ssh.DiscardRequests(requests)
		server, err := sftp.NewServer(channel, sftp.WithServerWorkingDirectory("/home/portmate"))
		if err != nil {
			return
		}
		defer server.Close()
		if err := server.Serve(); err != nil && !errors.Is(err, io.EOF) {
			log.Printf("SFTP session failed: %v", err)
		}
		return
	}
}

func subsystemName(payload []byte) string {
	var request struct {
		Name string
	}
	if err := ssh.Unmarshal(payload, &request); err != nil {
		return ""
	}
	return request.Name
}
