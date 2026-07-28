package dev.portmate.compat;

import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.List;
import java.util.concurrent.CountDownLatch;

import org.apache.sshd.common.file.virtualfs.VirtualFileSystemFactory;
import org.apache.sshd.server.SshServer;
import org.apache.sshd.server.auth.UserAuthFactory;
import org.apache.sshd.server.auth.gss.GSSAuthenticator;
import org.apache.sshd.server.auth.gss.UserAuthGSSFactory;
import org.apache.sshd.server.auth.password.UserAuthPasswordFactory;
import org.apache.sshd.server.keyprovider.SimpleGeneratorHostKeyProvider;
import org.apache.sshd.server.session.ServerSession;
import org.apache.sshd.server.shell.InteractiveProcessShellFactory;
import org.apache.sshd.server.shell.ProcessShellCommandFactory;
import org.apache.sshd.sftp.server.SftpSubsystemFactory;

public final class ApacheMinaGssapiServer {
    private static final String VERSION = "Apache MINA SSHD 2.19.0";
    private static final String USERNAME = "portmate";
    private static final String CLIENT_PRINCIPAL = "portmate@PORTMATE.TEST";
    private static final String SERVICE_PRINCIPAL = "host/localhost@PORTMATE.TEST";

    private ApacheMinaGssapiServer() {
    }

    public static void main(String[] args) throws Exception {
        if (args.length == 1 && "--version".equals(args[0])) {
            System.out.println(VERSION);
            return;
        }

        String authentication = envChoice("PORTMATE_GSSAPI_AUTH", "yes", List.of("yes", "no"));
        String sftpMode = envChoice(
                "PORTMATE_GSSAPI_SFTP",
                "normal",
                List.of("normal", "rejected", "operation-denied"));
        Path state = Path.of("/var/lib/portmate").toAbsolutePath().normalize();
        Path home = Path.of("operation-denied".equals(sftpMode)
                ? "/srv/portmate/blocked"
                : "/srv/portmate/home/portmate").toAbsolutePath().normalize();
        Files.createDirectories(state);

        SimpleGeneratorHostKeyProvider hostKeys =
                new SimpleGeneratorHostKeyProvider(state.resolve("hostkey.ser"));
        hostKeys.setAlgorithm("RSA");
        hostKeys.setKeySize(3072);

        SshServer server = SshServer.setUpDefaultServer();
        server.setHost("0.0.0.0");
        server.setPort(2222);
        server.setKeyPairProvider(hostKeys);
        server.setPasswordAuthenticator((username, password, session) ->
                USERNAME.equals(username) && "portmate".equals(password));
        server.setFileSystemFactory(new VirtualFileSystemFactory(home));
        server.setShellFactory(InteractiveProcessShellFactory.INSTANCE);
        server.setCommandFactory(ProcessShellCommandFactory.INSTANCE);

        List<UserAuthFactory> authFactories = new ArrayList<>();
        if ("yes".equals(authentication)) {
            GSSAuthenticator gssAuthenticator = new GSSAuthenticator() {
                @Override
                public boolean validateInitialUser(ServerSession session, String user) {
                    return USERNAME.equals(user);
                }

                @Override
                public boolean validateIdentity(ServerSession session, String identity) {
                    return CLIENT_PRINCIPAL.equalsIgnoreCase(identity);
                }
            };
            gssAuthenticator.setServicePrincipalName(SERVICE_PRINCIPAL);
            gssAuthenticator.setKeytabFile("/etc/krb5.keytab");
            server.setGSSAuthenticator(gssAuthenticator);
            authFactories.add(UserAuthGSSFactory.INSTANCE);
        }
        authFactories.add(UserAuthPasswordFactory.INSTANCE);
        server.setUserAuthFactories(authFactories);

        if (!"rejected".equals(sftpMode)) {
            server.setSubsystemFactories(List.of(new SftpSubsystemFactory.Builder().build()));
        }
        server.start();

        Runtime.getRuntime().addShutdownHook(new Thread(() -> {
            try {
                server.stop(true);
            } catch (Exception error) {
                System.err.println("Apache MINA GSSAPI shutdown failed: " + error.getMessage());
            }
        }, "apache-mina-gssapi-shutdown"));

        new CountDownLatch(1).await();
    }

    private static String envChoice(String name, String fallback, List<String> choices) {
        String value = System.getenv().getOrDefault(name, fallback);
        if (!choices.contains(value)) {
            throw new IllegalArgumentException(name + " has unsupported value " + value);
        }
        return value;
    }
}
