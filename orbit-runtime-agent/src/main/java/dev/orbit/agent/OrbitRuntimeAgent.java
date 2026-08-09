package dev.orbit.agent;

import java.lang.instrument.Instrumentation;
import java.nio.charset.StandardCharsets;
import java.nio.file.Path;
import java.util.Base64;

/** Entrypoint injected when Orbit wraps an Orbit Launcher start command. */
public final class OrbitRuntimeAgent {
    private OrbitRuntimeAgent() {}

    public static void premain(String arguments, Instrumentation instrumentation) {
        try {
            var options = AgentOptions.parse(arguments);
            Recorder.configure(options.instanceRoot(), options.sessionFile());
            instrumentation.addTransformer(new FileCallTransformer());
        } catch (Throwable error) {
            System.err.println("[orbit-runtime-agent] disabled: " + error.getMessage());
        }
    }

    private record AgentOptions(Path instanceRoot, Path sessionFile) {
        private static AgentOptions parse(String value) {
            if (value == null || value.isBlank()) {
                throw new IllegalArgumentException("missing agent arguments");
            }
            Path root = null;
            Path session = null;
            for (String item : value.split(";")) {
                int separator = item.indexOf('=');
                if (separator <= 0) {
                    throw new IllegalArgumentException("invalid agent argument");
                }
                String key = item.substring(0, separator);
                String decoded = new String(
                    Base64.getUrlDecoder().decode(item.substring(separator + 1)),
                    StandardCharsets.UTF_8
                );
                switch (key) {
                    case "root" -> root = Path.of(decoded).toAbsolutePath().normalize();
                    case "session" -> session = Path.of(decoded).toAbsolutePath().normalize();
                    default -> throw new IllegalArgumentException("unknown agent argument: " + key);
                }
            }
            if (root == null || session == null) {
                throw new IllegalArgumentException("root and session are required");
            }
            return new AgentOptions(root, session);
        }
    }
}
