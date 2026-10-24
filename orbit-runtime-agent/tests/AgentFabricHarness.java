import java.io.File;
import java.lang.reflect.InvocationTargetException;
import java.lang.reflect.Method;
import java.nio.file.Path;
import java.nio.file.Paths;
import java.util.ArrayList;
import java.util.Collection;
import java.util.List;
import java.util.regex.Pattern;

import net.fabricmc.api.EnvType;
import net.fabricmc.loader.impl.game.GameProvider;

/** Exercises real Fabric Knot class loaders, physical sources and helper isolation. */
public final class AgentFabricHarness {
    private AgentFabricHarness() {}

    public static void main(String[] arguments) throws Exception {
        Path fixture = Paths.get(arguments[0]).toRealPath();
        boolean compatibility = Boolean.parseBoolean(arguments[2]);
        Object knot = invokeKnot(
            null, "create", new Class<?>[] {boolean.class, boolean.class, EnvType.class, GameProvider.class},
            compatibility, false, EnvType.SERVER, null
        );
        String libraries = System.getProperty("fabric.systemLibraries");
        if (libraries == null || libraries.isEmpty()) {
            throw new AssertionError("Agent was not registered in Fabric system libraries");
        }
        List<Path> parentSources = new ArrayList<Path>();
        for (String library : libraries.split(Pattern.quote(File.pathSeparator))) {
            parentSources.add(Paths.get(library).toRealPath());
        }
        invokeKnot(knot, "setValidParentClassPath", new Class<?>[] {Collection.class}, parentSources);
        invokeKnot(knot, "addCodeSource", new Class<?>[] {Path.class}, fixture);
        ClassLoader loader = (ClassLoader) invokeKnot(knot, "getClassLoader", new Class<?>[0]);
        Class<?> type = loader.loadClass("AgentFixture");
        if (type.getClassLoader() != loader) {
            throw new AssertionError("Fixture bypassed the Fabric class loader");
        }
        Path source = Paths.get(type.getProtectionDomain().getCodeSource().getLocation().toURI()).toRealPath();
        if (!source.equals(fixture)) {
            throw new AssertionError("Fabric did not preserve the physical fixture JAR source: " + source);
        }
        try {
            type.getMethod("main", String[].class).invoke(null, (Object) new String[] {arguments[1]});
        } catch (InvocationTargetException error) {
            Throwable cause = error.getCause();
            if (cause instanceof Exception) throw (Exception) cause;
            if (cause instanceof Error) throw (Error) cause;
            throw error;
        }
    }

    // Fabric signs its JARs. Keep the harness outside its packages and reflect
    // only the factory/interface under test instead of changing those JARs.
    private static Object invokeKnot(Object target, String name, Class<?>[] parameters, Object... arguments)
        throws ReflectiveOperationException {
        Class<?> type = Class.forName("net.fabricmc.loader.impl.launch.knot.KnotClassLoaderInterface");
        Method method = type.getDeclaredMethod(name, parameters);
        method.setAccessible(true);
        return method.invoke(target, arguments);
    }
}
