import java.io.IOException;
import java.nio.charset.StandardCharsets;
import java.nio.file.DirectoryNotEmptyException;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.Paths;
import java.nio.file.StandardCopyOption;
import java.util.ArrayList;
import java.util.Comparator;
import java.util.List;
import java.util.stream.Stream;

/** Relocates the bundled ASM classes without requiring a second bytecode library. */
public final class RelocateAsm {
    private static final String SOURCE_PATH = "org/objectweb/asm";
    private static final String TARGET_PATH = "dev/orbit/shd/asm";
    private static final byte[] SOURCE_BYTES = SOURCE_PATH.getBytes(StandardCharsets.US_ASCII);
    private static final byte[] TARGET_BYTES = TARGET_PATH.getBytes(StandardCharsets.US_ASCII);
    private static final byte[] SOURCE_DOTTED = "org.objectweb.asm".getBytes(StandardCharsets.US_ASCII);
    private static final byte[] TARGET_DOTTED = "dev.orbit.shd.asm".getBytes(StandardCharsets.US_ASCII);

    private RelocateAsm() {}

    public static void main(String[] arguments) throws Exception {
        if (arguments.length != 1) {
            throw new IllegalArgumentException("usage: RelocateAsm <classes-directory>");
        }
        if (SOURCE_BYTES.length != TARGET_BYTES.length) {
            throw new IllegalStateException("relocation paths must have equal byte lengths");
        }
        if (SOURCE_DOTTED.length != TARGET_DOTTED.length) {
            throw new IllegalStateException("dotted relocation names must have equal byte lengths");
        }

        Path root = Paths.get(arguments[0]).toAbsolutePath().normalize();
        if (!Files.isDirectory(root)) {
            throw new IllegalArgumentException("classes directory does not exist: " + root);
        }

        int replacements = rewriteClassConstants(root);
        if (replacements == 0) {
            throw new IllegalStateException("no ASM class references were relocated");
        }
        relocateEntries(root);
        removeEmptyDirectories(root);

        if (containsSourceEntry(root)) {
            throw new IllegalStateException("unrelocated ASM entries remain under " + root);
        }
        Path relocatedReader = root.resolve(TARGET_PATH).resolve("ClassReader.class");
        if (!Files.isRegularFile(relocatedReader)) {
            throw new IllegalStateException("relocated ASM ClassReader is missing: " + relocatedReader);
        }
    }

    private static int rewriteClassConstants(Path root) throws IOException {
        List<Path> classes = new ArrayList<>();
        try (Stream<Path> paths = Files.walk(root)) {
            paths.filter(Files::isRegularFile)
                    .filter(path -> path.getFileName().toString().endsWith(".class"))
                    .forEach(classes::add);
        }

        int replacements = 0;
        for (Path path : classes) {
            byte[] content = Files.readAllBytes(path);
            int inFile = replace(content, SOURCE_BYTES, TARGET_BYTES);
            inFile += replace(content, SOURCE_DOTTED, TARGET_DOTTED);
            if (inFile > 0) {
                Files.write(path, content);
                replacements += inFile;
            }
        }
        return replacements;
    }

    private static int replace(byte[] content, byte[] source, byte[] target) {
        int replacements = 0;
        for (int index = 0; index <= content.length - source.length; index++) {
            if (!matches(content, index, source)) {
                continue;
            }
            System.arraycopy(target, 0, content, index, target.length);
            replacements++;
            index += source.length - 1;
        }
        return replacements;
    }

    private static boolean matches(byte[] content, int offset, byte[] expected) {
        for (int index = 0; index < expected.length; index++) {
            if (content[offset + index] != expected[index]) {
                return false;
            }
        }
        return true;
    }

    private static void relocateEntries(Path root) throws IOException {
        List<Path> sources = new ArrayList<>();
        try (Stream<Path> paths = Files.walk(root)) {
            paths.filter(Files::isRegularFile)
                    .filter(path -> normalized(root.relativize(path)).contains(SOURCE_PATH))
                    .forEach(sources::add);
        }

        for (Path source : sources) {
            String relative = normalized(root.relativize(source));
            Path destination = root.resolve(relative.replace(SOURCE_PATH, TARGET_PATH));
            Files.createDirectories(destination.getParent());
            Files.move(source, destination, StandardCopyOption.REPLACE_EXISTING);
        }
    }

    private static boolean containsSourceEntry(Path root) throws IOException {
        try (Stream<Path> paths = Files.walk(root)) {
            return paths.anyMatch(path -> normalized(root.relativize(path)).contains(SOURCE_PATH));
        }
    }

    private static void removeEmptyDirectories(Path root) throws IOException {
        List<Path> directories = new ArrayList<>();
        try (Stream<Path> paths = Files.walk(root)) {
            paths.filter(Files::isDirectory).forEach(directories::add);
        }
        directories.sort(Comparator.reverseOrder());
        for (Path directory : directories) {
            if (directory.equals(root)) {
                continue;
            }
            try {
                Files.delete(directory);
            } catch (DirectoryNotEmptyException ignored) {
                // Expected for directories that still contain packaged classes.
            }
        }
    }

    private static String normalized(Path path) {
        return path.toString().replace('\\', '/');
    }
}
