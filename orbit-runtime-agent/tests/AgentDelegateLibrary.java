import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Paths;

/** Shared helper whose file operation should be attributed to its caller. */
public final class AgentDelegateLibrary {
    private AgentDelegateLibrary() {}

    public static void write(String instance) throws Exception {
        Files.write(
            Paths.get(instance).resolve("config/delegated.bin"),
            "delegated".getBytes(StandardCharsets.UTF_8)
        );
    }
}
