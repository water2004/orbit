import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Paths;

public final class AgentOwnerA {
    private AgentOwnerA() {}

    public static void main(String[] arguments) throws Exception {
        Files.write(
            Paths.get(arguments[0]).resolve("config/last-writer.bin"),
            "owner-a".getBytes(StandardCharsets.UTF_8)
        );
    }
}
