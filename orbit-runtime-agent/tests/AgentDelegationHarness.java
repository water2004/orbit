import java.lang.reflect.InvocationTargetException;
import java.net.URL;
import java.net.URLClassLoader;
import java.nio.file.Path;
import java.nio.file.Paths;

/** Loads a consumer and its helper from separate managed package artifacts. */
public final class AgentDelegationHarness {
    private AgentDelegationHarness() {}

    public static void main(String[] arguments) throws Exception {
        Path consumer = Paths.get(arguments[0]);
        Path library = Paths.get(arguments[1]);
        try (URLClassLoader loader = new URLClassLoader(
            new URL[] {
                consumer.toAbsolutePath().normalize().toUri().toURL(),
                library.toAbsolutePath().normalize().toUri().toURL()
            },
            null
        )) {
            try {
                Class.forName("AgentDelegateConsumer", true, loader)
                    .getMethod("main", String[].class)
                    .invoke(null, (Object) new String[] {arguments[2]});
            } catch (InvocationTargetException error) {
                Throwable cause = error.getCause();
                if (cause instanceof Exception) throw (Exception) cause;
                if (cause instanceof Error) throw (Error) cause;
                throw error;
            }
        }
    }
}
