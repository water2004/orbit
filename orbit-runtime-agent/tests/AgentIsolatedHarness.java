import java.lang.reflect.InvocationTargetException;
import java.net.URL;
import java.net.URLClassLoader;
import java.nio.file.Path;

/** Loads the fixture without the application class loader, like a mod loader does. */
public final class AgentIsolatedHarness {
    private AgentIsolatedHarness() {}

    public static void main(String[] arguments) throws Exception {
        Path fixture = Path.of(arguments[0]).toAbsolutePath().normalize();
        String instance = arguments[1];
        try (var loader = new URLClassLoader(
            new URL[] {fixture.toUri().toURL()},
            ClassLoader.getPlatformClassLoader()
        )) {
            Class<?> type = Class.forName("AgentFixture", true, loader);
            try {
                type.getMethod("main", String[].class)
                    .invoke(null, (Object) new String[] {instance});
            } catch (InvocationTargetException error) {
                Throwable cause = error.getCause();
                if (cause instanceof Exception exception) {
                    throw exception;
                }
                if (cause instanceof Error nested) {
                    throw nested;
                }
                throw error;
            }
        }
    }
}
