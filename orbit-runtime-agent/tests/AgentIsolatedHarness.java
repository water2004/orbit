import java.lang.reflect.InvocationTargetException;
import java.net.URL;
import java.net.URLClassLoader;
import java.nio.file.Path;
import java.nio.file.Paths;

/** Loads the fixture without the application class loader, like a mod loader does. */
public final class AgentIsolatedHarness {
    private AgentIsolatedHarness() {}

    public static void main(String[] arguments) throws Exception {
        Path fixture = Paths.get(arguments[0]).toAbsolutePath().normalize();
        String instance = arguments[1];
        String mainClass = arguments.length >= 3 ? arguments[2] : "AgentFixture";
        try (URLClassLoader loader = new URLClassLoader(
            new URL[] {fixture.toUri().toURL()},
            null
        )) {
            Class<?> type = Class.forName(mainClass, true, loader);
            try {
                type.getMethod("main", String[].class)
                    .invoke(null, (Object) new String[] {instance});
            } catch (InvocationTargetException error) {
                Throwable cause = error.getCause();
                if (cause instanceof Exception) {
                    throw (Exception) cause;
                }
                if (cause instanceof Error) {
                    throw (Error) cause;
                }
                throw error;
            }
        }
    }
}
