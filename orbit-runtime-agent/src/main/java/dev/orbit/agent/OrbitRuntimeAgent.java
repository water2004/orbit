package dev.orbit.agent;

import java.io.File;
import java.lang.instrument.Instrumentation;
import java.net.URL;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.Paths;
import java.util.Base64;
import java.util.List;
import java.util.jar.JarFile;

/** Entrypoint injected when Orbit wraps an Orbit Launcher start command. */
public final class OrbitRuntimeAgent {
    private static JarFile bootstrapJar;

    private OrbitRuntimeAgent() {}

    public static void premain(String arguments, Instrumentation instrumentation) {
        try {
            AgentOptions options = AgentOptions.parse(arguments);
            String systemLibraryProperty = readSystemLibraryProperty(options.contextFile());
            exposeHelpersToEveryClassLoader(instrumentation, systemLibraryProperty);
            ModuleAccess.configure(instrumentation);
            Recorder.configure(options.instanceRoot(), options.sessionFile(), options.contextFile());
            instrumentation.addTransformer(new FileCallTransformer());
        } catch (Throwable error) {
            System.err.println("[orbit-runtime-agent] disabled: " + error.getMessage());
        }
    }

    private static String readSystemLibraryProperty(Path context) throws Exception {
        List<String> lines = Files.readAllLines(context, StandardCharsets.UTF_8);
        for (String line : lines) {
            String[] fields = line.split("\t", -1);
            if (fields.length == 3 && fields[0].equals("system-library") && fields[2].equals("end")) {
                if (fields[1].equals("fabric.systemLibraries") || fields[1].equals("loader.systemLibraries")) {
                    return fields[1];
                }
                throw new IllegalArgumentException("invalid Loader system-library property");
            }
        }
        return null;
    }

    private static void exposeHelpersToEveryClassLoader(
        Instrumentation instrumentation,
        String systemLibraryProperty
    ) throws Exception {
        URL source = OrbitRuntimeAgent.class.getProtectionDomain().getCodeSource().getLocation();
        if (source == null) {
            throw new IllegalStateException("agent JAR location is unavailable");
        }
        Path path = Paths.get(source.toURI()).toAbsolutePath().normalize();
        if (!Files.isRegularFile(path)) {
            throw new IllegalStateException("agent was not loaded from a regular JAR");
        }
        if (systemLibraryProperty != null) {
            appendPathProperty(systemLibraryProperty, path);
        }
        bootstrapJar = new JarFile(path.toFile());
        instrumentation.appendToBootstrapClassLoaderSearch(bootstrapJar);
    }

    private static void appendPathProperty(String key, Path path) {
        String normalized = path.toString();
        String existing = System.getProperty(key);
        if (existing == null || existing.trim().isEmpty()) {
            System.setProperty(key, normalized);
            return;
        }
        for (String item : existing.split(java.util.regex.Pattern.quote(File.pathSeparator))) {
            if (Paths.get(item).toAbsolutePath().normalize().equals(path)) {
                return;
            }
        }
        System.setProperty(key, existing + File.pathSeparator + normalized);
    }

    private static final class AgentOptions {
        private final Path instanceRoot;
        private final Path sessionFile;
        private final Path contextFile;

        private AgentOptions(Path instanceRoot, Path sessionFile, Path contextFile) {
            this.instanceRoot = instanceRoot;
            this.sessionFile = sessionFile;
            this.contextFile = contextFile;
        }

        private Path instanceRoot() { return instanceRoot; }
        private Path sessionFile() { return sessionFile; }
        private Path contextFile() { return contextFile; }

        private static AgentOptions parse(String value) {
            if (value == null || value.trim().isEmpty()) {
                throw new IllegalArgumentException("missing agent arguments");
            }
            Path root = null;
            Path session = null;
            Path context = null;
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
                if (key.equals("root")) {
                    root = Paths.get(decoded).toAbsolutePath().normalize();
                } else if (key.equals("session")) {
                    session = Paths.get(decoded).toAbsolutePath().normalize();
                } else if (key.equals("context")) {
                    context = Paths.get(decoded).toAbsolutePath().normalize();
                } else {
                    throw new IllegalArgumentException("unknown agent argument: " + key);
                }
            }
            if (root == null || session == null || context == null) {
                throw new IllegalArgumentException("root, session and context are required");
            }
            return new AgentOptions(root, session, context);
        }
    }
}
