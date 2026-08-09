import java.lang.reflect.InvocationTargetException;
import java.net.URL;
import java.net.URLClassLoader;
import java.nio.file.Path;
import java.nio.file.Paths;

/** Executes A, B, then A again so the final file owner must return to A. */
public final class AgentOwnershipHarness {
    private AgentOwnershipHarness() {}

    public static void main(String[] arguments) throws Exception {
        String instance = arguments[2];
        invoke(Paths.get(arguments[0]), "AgentOwnerA", instance);
        invoke(Paths.get(arguments[1]), "AgentOwnerB", instance);
        invoke(Paths.get(arguments[0]), "AgentOwnerA", instance);
    }

    private static void invoke(Path jar, String className, String instance) throws Exception {
        try (URLClassLoader loader = new URLClassLoader(
            new URL[] {jar.toAbsolutePath().normalize().toUri().toURL()},
            null
        )) {
            try {
                Class.forName(className, true, loader)
                    .getMethod("main", String[].class)
                    .invoke(null, (Object) new String[] {instance});
            } catch (InvocationTargetException error) {
                Throwable cause = error.getCause();
                if (cause instanceof Exception) throw (Exception) cause;
                if (cause instanceof Error) throw (Error) cause;
                throw error;
            }
        }
    }
}
