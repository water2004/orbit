import java.io.FileOutputStream;
import java.net.URI;
import java.nio.charset.StandardCharsets;
import java.nio.file.FileSystem;
import java.nio.file.FileSystems;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.Map;

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

        // Archive entries are virtual paths, not independently purgeable
        // instance data. Only the physical archive may ever be observed.
        Path archive = root.resolve("config/agent-fixture.zip");
        URI archiveUri = URI.create("jar:" + archive.toUri());
        try (FileSystem zip = FileSystems.newFileSystem(archiveUri, Map.of("create", "true"))) {
            Path virtualTree = zip.getPath("/META-INF/generated");
            Files.createDirectories(virtualTree);
            Files.writeString(virtualTree.resolve("entry.txt"), "virtual");
            Files.readString(virtualTree.resolve("entry.txt"));
        }

        // Physical writes outside the instance remain explicit external
        // ownership; they must not be confused with virtual archive entries.
        Path outside = root.getParent().resolve("agent-fixture-outside.txt");
        Files.writeString(outside, "outside");
        Files.readString(outside);
    }
}
