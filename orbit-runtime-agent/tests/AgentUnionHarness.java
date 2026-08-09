import java.lang.reflect.InvocationTargetException;
import java.net.URI;
import java.net.URL;
import java.net.URLConnection;
import java.net.URLStreamHandler;
import java.nio.file.FileSystem;
import java.nio.file.FileSystems;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.Paths;
import java.security.CodeSource;
import java.security.ProtectionDomain;
import java.security.cert.Certificate;
import java.util.HashMap;
import java.util.Map;
import java.util.function.BiPredicate;

/** Defines the fixture with the real SecureModules union CodeSource used by Forge. */
public final class AgentUnionHarness {
    private AgentUnionHarness() {}

    public static void main(String[] arguments) throws Exception {
        final Path fixture = Paths.get(arguments[0]).toAbsolutePath().normalize();
        final String instance = arguments[1];
        URI source = URI.create("union:" + fixture.toUri() + "!/");
        Map<String, Object> environment = new HashMap<String, Object>();
        environment.put("filter", new BiPredicate<String, String>() {
            @Override
            public boolean test(String path, String base) { return true; }
        });
        try (FileSystem union = FileSystems.newFileSystem(source, environment)) {
            Path classFile = union.getPath("/AgentFixture.class");
            byte[] bytes = Files.readAllBytes(classFile);
            URL codeSource = new URL(null, classFile.toUri().toString(), new URLStreamHandler() {
                @Override
                protected URLConnection openConnection(URL ignored) {
                    throw new UnsupportedOperationException();
                }
            });
            System.out.println("union-code-source=" + codeSource);
            FileSystem resolved = Paths.get(codeSource.toURI()).getFileSystem();
            System.out.println("union-provider=" + resolved.getClass().getName());
            UnionClassLoader loader = new UnionClassLoader(codeSource);
            Class<?> type = loader.define(bytes);
            Object owner = Class.forName("dev.orbit.agent.Recorder", false, null)
                .getMethod("ownerFor", ProtectionDomain.class)
                .invoke(null, type.getProtectionDomain());
            System.out.println("union-owner=" + owner);
            try {
                type.getMethod("main", String[].class)
                    .invoke(null, (Object) new String[] {instance});
            } catch (InvocationTargetException error) {
                Throwable cause = error.getCause();
                if (cause instanceof Exception) throw (Exception) cause;
                if (cause instanceof Error) throw (Error) cause;
                throw error;
            }
        }
    }

    private static final class UnionClassLoader extends ClassLoader {
        private final ProtectionDomain domain;

        private UnionClassLoader(URL source) {
            super(null);
            this.domain = new ProtectionDomain(
                new CodeSource(source, (Certificate[]) null),
                null,
                this,
                null
            );
        }

        private Class<?> define(byte[] bytes) {
            return defineClass("AgentFixture", bytes, 0, bytes.length, domain);
        }
    }
}
