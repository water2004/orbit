import java.io.FileWriter;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.Paths;

/** Exercises mutating APIs added after the Agent's Java 8 baseline. */
public final class AgentModernFixture {
    private AgentModernFixture() {}

    public static void main(String[] arguments) throws Exception {
        Path config = Paths.get(arguments[0]).resolve("config");
        Files.writeString(config.resolve("modern-write-string.txt"), "writeString");
        try (FileWriter writer = new FileWriter(
            config.resolve("modern-file-writer.txt").toFile(),
            StandardCharsets.UTF_8
        )) {
            writer.write("charset constructor");
        }
    }
}
