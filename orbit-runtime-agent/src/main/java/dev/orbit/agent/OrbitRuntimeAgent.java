package dev.orbit.agent;

import java.io.File;
import java.lang.instrument.Instrumentation;
import java.nio.charset.StandardCharsets;
import java.nio.file.Path;
import java.util.Base64;
import java.util.jar.JarFile;

/** Entrypoint injected when Orbit wraps an Orbit Launcher start command. */
public final class OrbitRuntimeAgent {
    // Instrumentation keeps using this JAR as a bootstrap search source for
    // the life of the JVM, so Orbit owns the handle until process exit.
    private static JarFile bootstrapJar;

    private OrbitRuntimeAgent() {}

    public static void premain(String arguments, Instrumentation instrumentation) {
        try {
            var options = AgentOptions.parse(arguments);
            exposeHelpersToEveryClassLoader(instrumentation);
            Recorder.configure(options.instanceRoot(), options.sessionFile());
            instrumentation.addTransformer(new FileCallTransformer());
        } catch (Throwable error) {
            System.err.println("[orbit-runtime-agent] disabled: " + error.getMessage());
        }
    }

    private static void exposeHelpersToEveryClassLoader(Instrumentation instrumentation) throws Exception {
        var source = OrbitRuntimeAgent.class.getProtectionDomain().getCodeSource();
        if (source == null) {
            throw new IllegalStateException("agent JAR location is unavailable");
        }
        Path path = Path.of(source.getLocation().toURI()).toAbsolutePath().normalize();
        if (!java.nio.file.Files.isRegularFile(path)) {
            throw new IllegalStateException("agent was not loaded from a regular JAR");
        }
        // Fabric and Quilt intentionally restrict which parent-loader code
        // sources their game loader may access. These are their documented
        // system-library properties; declaring the Agent as a system library
        // keeps it outside the mod/game class path while allowing delegation
        // to the bootstrap-visible helper classes.
        appendPathProperty("fabric.systemLibraries", path);
        appendPathProperty("loader.systemLibraries", path);
        bootstrapJar = new JarFile(path.toFile());
        instrumentation.appendToBootstrapClassLoaderSearch(bootstrapJar);
    }

    private static void appendPathProperty(String key, Path path) {
        String normalized = path.toString();
        String existing = System.getProperty(key);
        if (existing == null || existing.isBlank()) {
            System.setProperty(key, normalized);
            return;
        }
        for (String item : existing.split(java.util.regex.Pattern.quote(File.pathSeparator))) {
            if (Path.of(item).toAbsolutePath().normalize().equals(path)) {
                return;
            }
        }
        System.setProperty(key, existing + File.pathSeparator + normalized);
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
