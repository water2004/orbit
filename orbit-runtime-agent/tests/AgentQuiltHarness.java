import java.lang.reflect.InvocationHandler;
import java.lang.reflect.InvocationTargetException;
import java.lang.reflect.Method;
import java.lang.reflect.Proxy;
import java.net.URL;
import java.net.URLConnection;
import java.net.URLStreamHandler;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.Paths;
import java.security.CodeSource;
import java.security.ProtectionDomain;
import java.security.cert.Certificate;
import java.util.Optional;

import org.quiltmc.loader.api.ModContainer;
import org.quiltmc.loader.api.ModMetadata;
import org.quiltmc.loader.impl.launch.common.QuiltCodeSource;

/** Exercises the public native module identity added by Quilt Loader 0.18.1. */
public final class AgentQuiltHarness {
    private AgentQuiltHarness() {}

    public static void main(String[] arguments) throws Exception {
        Path fixture = Paths.get(arguments[0]).toAbsolutePath().normalize();
        String instance = arguments[1];
        byte[] bytes;
        try (java.nio.file.FileSystem archive = java.nio.file.FileSystems.newFileSystem(fixture, (ClassLoader) null)) {
            bytes = Files.readAllBytes(archive.getPath("/AgentFixture.class"));
        }
        ModMetadata metadata = (ModMetadata) Proxy.newProxyInstance(
            AgentQuiltHarness.class.getClassLoader(),
            new Class<?>[] {ModMetadata.class},
            new ConstantMethod("id", "agent-fixture")
        );
        ModContainer container = (ModContainer) Proxy.newProxyInstance(
            AgentQuiltHarness.class.getClassLoader(),
            new Class<?>[] {ModContainer.class},
            new ConstantMethod("metadata", metadata)
        );
        URL virtualSource = new URL(null, "quilt:/virtual/agent-fixture", new URLStreamHandler() {
            @Override
            protected URLConnection openConnection(URL ignored) {
                throw new UnsupportedOperationException();
            }
        });
        QuiltSource source = new QuiltSource(virtualSource, container);
        QuiltClassLoader loader = new QuiltClassLoader(source);
        Class<?> type = loader.define(bytes);
        Object owner = Class.forName("dev.orbit.agent.Recorder", false, null)
            .getMethod("ownerFor", ProtectionDomain.class)
            .invoke(null, type.getProtectionDomain());
        System.out.println("quilt-owner=" + owner);
        try {
            type.getMethod("main", String[].class).invoke(null, (Object) new String[] {instance});
        } catch (InvocationTargetException error) {
            Throwable cause = error.getCause();
            if (cause instanceof Exception) throw (Exception) cause;
            if (cause instanceof Error) throw (Error) cause;
            throw error;
        }
    }

    private static final class QuiltSource extends CodeSource implements QuiltCodeSource {
        private final ModContainer container;

        private QuiltSource(URL source, ModContainer container) {
            super(source, (Certificate[]) null);
            this.container = container;
        }

        @Override
        public Optional<ModContainer> getQuiltMod() {
            return Optional.of(container);
        }
    }

    private static final class QuiltClassLoader extends ClassLoader {
        private final ProtectionDomain domain;

        private QuiltClassLoader(CodeSource source) {
            super(AgentQuiltHarness.class.getClassLoader());
            domain = new ProtectionDomain(source, null, this, null);
        }

        private Class<?> define(byte[] bytes) {
            return defineClass("AgentFixture", bytes, 0, bytes.length, domain);
        }
    }

    private static final class ConstantMethod implements InvocationHandler {
        private final String name;
        private final Object value;

        private ConstantMethod(String name, Object value) {
            this.name = name;
            this.value = value;
        }

        @Override
        public Object invoke(Object proxy, Method method, Object[] arguments) {
            if (method.getName().equals(name)) return value;
            if (method.getName().equals("toString")) return "Orbit test proxy";
            if (method.getReturnType().equals(boolean.class)) return Boolean.FALSE;
            if (method.getReturnType().equals(int.class)) return Integer.valueOf(0);
            return null;
        }
    }
}
