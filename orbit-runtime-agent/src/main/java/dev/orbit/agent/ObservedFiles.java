package dev.orbit.agent;

import java.io.BufferedWriter;
import java.io.IOException;
import java.io.InputStream;
import java.io.OutputStream;
import java.nio.charset.Charset;
import java.nio.channels.SeekableByteChannel;
import java.nio.file.CopyOption;
import java.nio.file.Files;
import java.nio.file.OpenOption;
import java.nio.file.Path;
import java.nio.file.attribute.FileAttribute;
import java.nio.file.attribute.FileTime;
import java.nio.file.attribute.UserPrincipal;
import java.util.Set;
import java.util.Arrays;
import java.util.Collections;
import java.util.HashSet;

/** Exact-signature wrappers for mutating {@link Files} operations. */
public final class ObservedFiles {
    private static final Set<String> SUPPORTED = Collections.unmodifiableSet(new HashSet<String>(Arrays.asList(
        "newOutputStream(Ljava/nio/file/Path;[Ljava/nio/file/OpenOption;)Ljava/io/OutputStream;",
        "newByteChannel(Ljava/nio/file/Path;[Ljava/nio/file/OpenOption;)Ljava/nio/channels/SeekableByteChannel;",
        "newByteChannel(Ljava/nio/file/Path;Ljava/util/Set;[Ljava/nio/file/attribute/FileAttribute;)Ljava/nio/channels/SeekableByteChannel;",
        "newBufferedWriter(Ljava/nio/file/Path;Ljava/nio/charset/Charset;[Ljava/nio/file/OpenOption;)Ljava/io/BufferedWriter;",
        "write(Ljava/nio/file/Path;[B[Ljava/nio/file/OpenOption;)Ljava/nio/file/Path;",
        "write(Ljava/nio/file/Path;Ljava/lang/Iterable;[Ljava/nio/file/OpenOption;)Ljava/nio/file/Path;",
        "write(Ljava/nio/file/Path;Ljava/lang/Iterable;Ljava/nio/charset/Charset;[Ljava/nio/file/OpenOption;)Ljava/nio/file/Path;",
        "writeString(Ljava/nio/file/Path;Ljava/lang/CharSequence;[Ljava/nio/file/OpenOption;)Ljava/nio/file/Path;",
        "writeString(Ljava/nio/file/Path;Ljava/lang/CharSequence;Ljava/nio/charset/Charset;[Ljava/nio/file/OpenOption;)Ljava/nio/file/Path;",
        "createFile(Ljava/nio/file/Path;[Ljava/nio/file/attribute/FileAttribute;)Ljava/nio/file/Path;",
        "createDirectory(Ljava/nio/file/Path;[Ljava/nio/file/attribute/FileAttribute;)Ljava/nio/file/Path;",
        "createDirectories(Ljava/nio/file/Path;[Ljava/nio/file/attribute/FileAttribute;)Ljava/nio/file/Path;",
        "createTempFile(Ljava/nio/file/Path;Ljava/lang/String;Ljava/lang/String;[Ljava/nio/file/attribute/FileAttribute;)Ljava/nio/file/Path;",
        "createTempFile(Ljava/lang/String;Ljava/lang/String;[Ljava/nio/file/attribute/FileAttribute;)Ljava/nio/file/Path;",
        "createTempDirectory(Ljava/nio/file/Path;Ljava/lang/String;[Ljava/nio/file/attribute/FileAttribute;)Ljava/nio/file/Path;",
        "createTempDirectory(Ljava/lang/String;[Ljava/nio/file/attribute/FileAttribute;)Ljava/nio/file/Path;",
        "delete(Ljava/nio/file/Path;)V",
        "deleteIfExists(Ljava/nio/file/Path;)Z",
        "copy(Ljava/nio/file/Path;Ljava/nio/file/Path;[Ljava/nio/file/CopyOption;)Ljava/nio/file/Path;",
        "copy(Ljava/io/InputStream;Ljava/nio/file/Path;[Ljava/nio/file/CopyOption;)J",
        "move(Ljava/nio/file/Path;Ljava/nio/file/Path;[Ljava/nio/file/CopyOption;)Ljava/nio/file/Path;",
        "setLastModifiedTime(Ljava/nio/file/Path;Ljava/nio/file/attribute/FileTime;)Ljava/nio/file/Path;",
        "setOwner(Ljava/nio/file/Path;Ljava/nio/file/attribute/UserPrincipal;)Ljava/nio/file/Path;",
        "setAttribute(Ljava/nio/file/Path;Ljava/lang/String;Ljava/lang/Object;[Ljava/nio/file/LinkOption;)Ljava/nio/file/Path;"
    )));

    private ObservedFiles() {}

    public static boolean supports(String name, String descriptor) {
        return SUPPORTED.contains(name + descriptor);
    }

    private static boolean before(Path path) {
        return Files.exists(path);
    }

    private static <T> T write(Path path, String owner, IoSupplier<T> operation) throws IOException {
        if (Recorder.owns(path, owner)) {
            return operation.get();
        }
        boolean existed = before(path);
        T result = operation.get();
        Recorder.write(path, existed, owner);
        return result;
    }

    public static OutputStream newOutputStream(Path path, OpenOption[] options, String owner) throws IOException {
        return write(path, owner, () -> Files.newOutputStream(path, options));
    }

    public static SeekableByteChannel newByteChannel(Path path, OpenOption[] options, String owner) throws IOException {
        if (Recorder.owns(path, owner)) return Files.newByteChannel(path, options);
        boolean existed = before(path);
        SeekableByteChannel channel = Files.newByteChannel(path, options);
        observeOptions(path, existed, new HashSet<OpenOption>(Arrays.asList(options)), owner);
        return channel;
    }

    public static SeekableByteChannel newByteChannel(
        Path path,
        Set<? extends OpenOption> options,
        FileAttribute<?>[] attributes,
        String owner
    ) throws IOException {
        if (Recorder.owns(path, owner)) return Files.newByteChannel(path, options, attributes);
        boolean existed = before(path);
        SeekableByteChannel channel = Files.newByteChannel(path, options, attributes);
        observeOptions(path, existed, options, owner);
        return channel;
    }

    public static BufferedWriter newBufferedWriter(
        Path path,
        Charset charset,
        OpenOption[] options,
        String owner
    ) throws IOException {
        return write(path, owner, () -> Files.newBufferedWriter(path, charset, options));
    }

    public static Path write(Path path, byte[] bytes, OpenOption[] options, String owner) throws IOException {
        return write(path, owner, () -> Files.write(path, bytes, options));
    }

    public static Path write(
        Path path,
        Iterable<? extends CharSequence> lines,
        OpenOption[] options,
        String owner
    ) throws IOException {
        return write(path, owner, () -> Files.write(path, lines, options));
    }

    public static Path write(
        Path path,
        Iterable<? extends CharSequence> lines,
        Charset charset,
        OpenOption[] options,
        String owner
    ) throws IOException {
        return write(path, owner, () -> Files.write(path, lines, charset, options));
    }

    public static Path writeString(
        Path path,
        CharSequence value,
        OpenOption[] options,
        String owner
    ) throws IOException {
        return write(path, owner, () -> writeCharacters(path, value, java.nio.charset.StandardCharsets.UTF_8, options));
    }

    public static Path writeString(
        Path path,
        CharSequence value,
        Charset charset,
        OpenOption[] options,
        String owner
    ) throws IOException {
        return write(path, owner, () -> writeCharacters(path, value, charset, options));
    }

    private static Path writeCharacters(
        Path path,
        CharSequence value,
        Charset charset,
        OpenOption[] options
    ) throws IOException {
        try (BufferedWriter writer = Files.newBufferedWriter(path, charset, options)) {
            writer.append(value);
        }
        return path;
    }

    public static Path createFile(Path path, FileAttribute<?>[] attributes, String owner) throws IOException {
        Path result = Files.createFile(path, attributes);
        Recorder.write(path, false, owner);
        return result;
    }

    public static Path createDirectory(Path path, FileAttribute<?>[] attributes, String owner) throws IOException {
        Path result = Files.createDirectory(path, attributes);
        Recorder.tree(path, false, true, owner);
        return result;
    }

    public static Path createDirectories(Path path, FileAttribute<?>[] attributes, String owner) throws IOException {
        Path createdRoot = Recorder.firstOwnedMissingAncestor(path);
        Path result = Files.createDirectories(path, attributes);
        if (createdRoot != null) Recorder.tree(createdRoot, false, true, owner);
        return result;
    }

    public static Path createTempFile(
        Path directory,
        String prefix,
        String suffix,
        FileAttribute<?>[] attributes,
        String owner
    ) throws IOException {
        Path result = Files.createTempFile(directory, prefix, suffix, attributes);
        Recorder.write(result, false, owner);
        return result;
    }

    public static Path createTempFile(
        String prefix,
        String suffix,
        FileAttribute<?>[] attributes,
        String owner
    ) throws IOException {
        Path result = Files.createTempFile(prefix, suffix, attributes);
        Recorder.write(result, false, owner);
        return result;
    }

    public static Path createTempDirectory(
        Path directory,
        String prefix,
        FileAttribute<?>[] attributes,
        String owner
    ) throws IOException {
        Path result = Files.createTempDirectory(directory, prefix, attributes);
        Recorder.tree(result, false, true, owner);
        return result;
    }

    public static Path createTempDirectory(
        String prefix,
        FileAttribute<?>[] attributes,
        String owner
    ) throws IOException {
        Path result = Files.createTempDirectory(prefix, attributes);
        Recorder.tree(result, false, true, owner);
        return result;
    }

    public static void delete(Path path, String owner) throws IOException {
        boolean tree = Files.isDirectory(path);
        Files.delete(path);
        Recorder.delete(path, tree, owner);
    }

    public static boolean deleteIfExists(Path path, String owner) throws IOException {
        boolean tree = Files.isDirectory(path);
        boolean deleted = Files.deleteIfExists(path);
        if (deleted) Recorder.delete(path, tree, owner);
        return deleted;
    }

    public static Path copy(Path source, Path target, CopyOption[] options, String owner) throws IOException {
        boolean existed = before(target);
        Path result = Files.copy(source, target, options);
        Recorder.write(target, existed, owner);
        return result;
    }

    public static long copy(InputStream source, Path target, CopyOption[] options, String owner) throws IOException {
        boolean existed = before(target);
        long result = Files.copy(source, target, options);
        Recorder.write(target, existed, owner);
        return result;
    }

    public static Path move(Path source, Path target, CopyOption[] options, String owner) throws IOException {
        boolean sourceTree = Files.isDirectory(source);
        Path result = Files.move(source, target, options);
        Recorder.delete(source, sourceTree, owner);
        if (sourceTree) Recorder.tree(target, false, true, owner);
        else Recorder.write(target, false, owner);
        return result;
    }

    public static Path setLastModifiedTime(Path path, FileTime value, String owner) throws IOException {
        return write(path, owner, () -> Files.setLastModifiedTime(path, value));
    }

    public static Path setOwner(Path path, UserPrincipal value, String owner) throws IOException {
        return write(path, owner, () -> Files.setOwner(path, value));
    }

    public static Path setAttribute(
        Path path,
        String attribute,
        Object value,
        java.nio.file.LinkOption[] options,
        String owner
    ) throws IOException {
        return write(path, owner, () -> Files.setAttribute(path, attribute, value, options));
    }

    public static boolean fileCreateNewFile(java.io.File file, String owner) throws IOException {
        boolean created = file.createNewFile();
        if (created) Recorder.write(file.toPath(), false, owner);
        return created;
    }

    public static boolean fileDelete(java.io.File file, String owner) {
        boolean tree = file.isDirectory();
        boolean deleted = file.delete();
        if (deleted) Recorder.delete(file.toPath(), tree, owner);
        return deleted;
    }

    public static boolean fileMkdir(java.io.File file, String owner) {
        boolean created = file.mkdir();
        if (created) Recorder.tree(file.toPath(), false, true, owner);
        return created;
    }

    public static boolean fileMkdirs(java.io.File file, String owner) {
        Path createdRoot = Recorder.firstOwnedMissingAncestor(file.toPath());
        boolean created = file.mkdirs();
        if (created && createdRoot != null) Recorder.tree(createdRoot, false, true, owner);
        return created;
    }

    public static boolean fileRenameTo(java.io.File source, java.io.File target, String owner) {
        boolean sourceTree = source.isDirectory();
        boolean moved = source.renameTo(target);
        if (moved) {
            Recorder.delete(source.toPath(), sourceTree, owner);
            if (sourceTree) Recorder.tree(target.toPath(), false, true, owner);
            else Recorder.write(target.toPath(), false, owner);
        }
        return moved;
    }

    private static void observeOptions(
        Path path,
        boolean existed,
        Set<? extends OpenOption> options,
        String owner
    ) {
        boolean write = options.contains(java.nio.file.StandardOpenOption.WRITE)
            || options.contains(java.nio.file.StandardOpenOption.APPEND)
            || options.contains(java.nio.file.StandardOpenOption.CREATE)
            || options.contains(java.nio.file.StandardOpenOption.CREATE_NEW)
            || options.contains(java.nio.file.StandardOpenOption.DELETE_ON_CLOSE);
        if (write) Recorder.write(path, existed, owner);
    }

    @FunctionalInterface
    private interface IoSupplier<T> { T get() throws IOException; }
}
