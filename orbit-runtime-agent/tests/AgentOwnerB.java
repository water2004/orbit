import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Paths;

public final class AgentOwnerB {
    private AgentOwnerB() {}

    public static void main(String[] arguments) throws Exception {
        Files.write(
            Paths.get(arguments[0]).resolve("config/last-writer.bin"),
            "owner-b".getBytes(StandardCharsets.UTF_8)
        );
    }
}
