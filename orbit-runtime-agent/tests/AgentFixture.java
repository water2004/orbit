import java.io.FileOutputStream;
import java.net.URI;
import java.nio.charset.StandardCharsets;
import java.nio.file.FileSystem;
import java.nio.file.FileSystems;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.Paths;
import java.nio.file.StandardCopyOption;
import java.nio.file.StandardOpenOption;
import java.util.Map;

public final class AgentFixture {
    private AgentFixture() {}

    public static void main(String[] arguments) throws Exception {
        Path root = Paths.get(arguments[0]);
        Path tree = root.resolve("config/agent-fixture/database");
        Files.createDirectories(tree);
        for (int index = 0; index < 128; index++) {
            Files.write(tree.resolve(index + ".bin"), ("value-" + index).getBytes(StandardCharsets.UTF_8));
        }
        // BlueMap's default file storage writes a .filepart and atomically
        // replaces the published tile. Both paths already belong to this
        // package tree and must not create per-tile ledger state.
        Path tilePart = tree.resolve("tile.prbm.gz.filepart");
        Files.write(
            tilePart,
            "tile".getBytes(StandardCharsets.UTF_8),
            StandardOpenOption.CREATE,
            StandardOpenOption.TRUNCATE_EXISTING,
            StandardOpenOption.WRITE
        );
        Files.move(
            tilePart,
            tree.resolve("tile.prbm.gz"),
            StandardCopyOption.ATOMIC_MOVE,
            StandardCopyOption.REPLACE_EXISTING
        );
        try (FileOutputStream output = new FileOutputStream(root.resolve("config/agent-fixture.properties").toFile())) {
            output.write("enabled=true".getBytes(StandardCharsets.UTF_8));
        }
        Files.readAllBytes(root.resolve("config/agent-fixture.properties"));

        // Archive entries are virtual paths, not independently purgeable
        // instance data. Only the physical archive may ever be observed.
        Path archive = root.resolve("config/agent-fixture.zip");
        URI archiveUri = URI.create("jar:" + archive.toUri());
        try (FileSystem zip = FileSystems.newFileSystem(archiveUri, java.util.Collections.singletonMap("create", "true"))) {
            Path virtualTree = zip.getPath("/META-INF/generated");
            Files.createDirectories(virtualTree);
            Files.write(virtualTree.resolve("entry.txt"), "virtual".getBytes(StandardCharsets.UTF_8));
            Files.readAllBytes(virtualTree.resolve("entry.txt"));
        }

        // Physical writes outside the instance remain explicit external
        // ownership; they must not be confused with virtual archive entries.
        Path outside = root.getParent().resolve("agent-fixture-outside.txt");
        Files.write(outside, "outside".getBytes(StandardCharsets.UTF_8));
        Files.readAllBytes(outside);

        // A transient path that was never published has no lasting ownership
        // effect and must disappear from the complete snapshot.
        Path quick = root.getParent().resolve("agent-fixture-quick.tmp");
        Files.write(quick, "quick".getBytes(StandardCharsets.UTF_8));
        Files.delete(quick);

        // Once a generation containing a creation may have been claimed by
        // Orbit, a later deletion must remain as an explicit tombstone.
        Path published = root.getParent().resolve("agent-fixture-published.tmp");
        Files.write(published, "published".getBytes(StandardCharsets.UTF_8));
        Thread.sleep(1500L);
        Files.delete(published);
    }
}
