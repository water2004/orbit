package dev.orbit.agent;

import java.lang.instrument.Instrumentation;
import java.lang.reflect.Method;
import java.net.URI;
import java.nio.file.FileSystem;
import java.nio.file.Path;
import java.nio.file.Paths;
import java.util.Collections;
import java.util.HashMap;
import java.util.HashSet;
import java.util.Map;
import java.util.Set;

/** Java 8-compatible facade for Java 9 module access and Forge union paths. */
public final class ModuleAccess {
    private static volatile Instrumentation instrumentation;
    private static final Set<Object> EXPORTED = Collections.synchronizedSet(new HashSet<Object>());

    private ModuleAccess() {}

    public static void configure(Instrumentation value) {
        instrumentation = value;
    }

    public static Path unionPrimaryPath(URI source) {
        try {
            Path unionPath = Paths.get(source);
            FileSystem fileSystem = unionPath.getFileSystem();
            Class<?> type = fileSystem.getClass();
            exportToAgentIfNecessary(type);
            Method method = type.getMethod("getPrimaryPath");
            return (Path) method.invoke(fileSystem);
        } catch (Throwable ignored) {
            return null;
        }
    }

    private static void exportToAgentIfNecessary(Class<?> type) throws Exception {
        Class<?> moduleType;
        try {
            moduleType = Class.forName("java.lang.Module");
        } catch (ClassNotFoundException ignored) {
            return;
        }
        Object sourceModule = Class.class.getMethod("getModule").invoke(type);
        Object targetModule = Class.class.getMethod("getModule").invoke(ModuleAccess.class);
        String packageName = type.getPackage().getName();
        boolean exported = ((Boolean) moduleType
            .getMethod("isExported", String.class, moduleType)
            .invoke(sourceModule, packageName, targetModule)).booleanValue();
        if (exported || !EXPORTED.add(sourceModule)) {
            return;
        }
        Map<String, Set<Object>> exports = new HashMap<String, Set<Object>>();
        exports.put(packageName, Collections.singleton(targetModule));
        Instrumentation.class.getMethod(
            "redefineModule",
            moduleType,
            Set.class,
            Map.class,
            Map.class,
            Set.class,
            Map.class
        ).invoke(
            instrumentation,
            sourceModule,
            Collections.emptySet(),
            exports,
            Collections.emptyMap(),
            Collections.emptySet(),
            Collections.emptyMap()
        );
    }
}
