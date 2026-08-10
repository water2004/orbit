import java.net.URL;
import java.util.Enumeration;

public final class AgentClasspathHarness {
    private AgentClasspathHarness() {}

    public static void main(String[] arguments) throws Exception {
        int publicAsm = count("org/objectweb/asm/ClassReader.class");
        int privateAsm = count("dev/orbit/shd/asm/ClassReader.class");
        if (publicAsm != 1) {
            throw new IllegalStateException(
                    "expected exactly one public ASM ClassReader resource, found " + publicAsm);
        }
        if (privateAsm != 1) {
            throw new IllegalStateException(
                    "expected exactly one relocated Agent ASM ClassReader resource, found " + privateAsm);
        }
    }

    private static int count(String resource) throws Exception {
        Enumeration<URL> resources = ClassLoader.getSystemResources(resource);
        int count = 0;
        while (resources.hasMoreElements()) {
            resources.nextElement();
            count++;
        }
        return count;
    }
}
