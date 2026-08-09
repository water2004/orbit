import java.io.FileOutputStream;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;

public final class AgentFixture {
    private AgentFixture() {}

    public static void main(String[] arguments) throws Exception {
        Path root = Path.of(arguments[0]);
        Path tree = root.resolve("config/agent-fixture/database");
        Files.createDirectories(tree);
        for (int index = 0; index < 128; index++) {
            Files.writeString(tree.resolve(index + ".bin"), "value-" + index);
        }
        try (var output = new FileOutputStream(root.resolve("config/agent-fixture.properties").toFile())) {
            output.write("enabled=true".getBytes(StandardCharsets.UTF_8));
        }
        Files.readAllBytes(root.resolve("config/agent-fixture.properties"));
    }
}
