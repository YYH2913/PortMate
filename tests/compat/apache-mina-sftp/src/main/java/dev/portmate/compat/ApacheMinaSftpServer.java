package dev.portmate.compat;

import java.nio.file.Files;
import java.nio.file.Path;
import java.util.List;
import java.util.concurrent.CountDownLatch;

import org.apache.sshd.common.file.virtualfs.VirtualFileSystemFactory;
import org.apache.sshd.server.SshServer;
import org.apache.sshd.server.keyprovider.SimpleGeneratorHostKeyProvider;
import org.apache.sshd.sftp.server.SftpSubsystemFactory;

public final class ApacheMinaSftpServer {
    private ApacheMinaSftpServer() {
    }

    public static void main(String[] args) throws Exception {
        Path root = Path.of("/srv/portmate").toAbsolutePath().normalize();
        Path state = Path.of("/var/lib/portmate").toAbsolutePath().normalize();
        Files.createDirectories(root.resolve("home/portmate"));
        Files.createDirectories(state);

        SimpleGeneratorHostKeyProvider hostKeys =
                new SimpleGeneratorHostKeyProvider(state.resolve("hostkey.ser"));
        hostKeys.setAlgorithm("RSA");
        hostKeys.setKeySize(3072);

        SshServer server = SshServer.setUpDefaultServer();
        server.setHost("0.0.0.0");
        server.setPort(22);
        server.setKeyPairProvider(hostKeys);
        server.setPasswordAuthenticator((username, password, session) ->
                "portmate".equals(username) && "portmate".equals(password));
        server.setFileSystemFactory(new VirtualFileSystemFactory(root));
        server.setSubsystemFactories(List.of(new SftpSubsystemFactory.Builder().build()));
        server.start();

        Runtime.getRuntime().addShutdownHook(new Thread(() -> {
            try {
                server.stop(true);
            } catch (Exception error) {
                System.err.println("Apache MINA SSHD shutdown failed: " + error.getMessage());
            }
        }, "apache-mina-sftp-shutdown"));

        new CountDownLatch(1).await();
    }
}
