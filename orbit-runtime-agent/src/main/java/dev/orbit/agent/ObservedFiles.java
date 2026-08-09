package dev.orbit.agent;

import java.io.BufferedReader;
import java.io.BufferedWriter;
import java.io.IOException;
import java.io.InputStream;
import java.io.OutputStream;
import java.nio.charset.Charset;
import java.nio.channels.SeekableByteChannel;
import java.nio.file.CopyOption;
import java.nio.file.DirectoryStream;
import java.nio.file.FileVisitOption;
import java.nio.file.Files;
import java.nio.file.LinkOption;
import java.nio.file.OpenOption;
import java.nio.file.Path;
import java.nio.file.attribute.FileAttribute;
import java.nio.file.attribute.FileTime;
import java.nio.file.attribute.UserPrincipal;
import java.util.List;
import java.util.Set;
import java.util.stream.Stream;

/** Exact-signature wrappers for common java.nio.file.Files operations. */
public final class ObservedFiles {
    private static final Set<String> SUPPORTED = Set.of(
        "newInputStream(Ljava/nio/file/Path;[Ljava/nio/file/OpenOption;)Ljava/io/InputStream;",
        "newOutputStream(Ljava/nio/file/Path;[Ljava/nio/file/OpenOption;)Ljava/io/OutputStream;",
        "newByteChannel(Ljava/nio/file/Path;[Ljava/nio/file/OpenOption;)Ljava/nio/channels/SeekableByteChannel;",
        "newByteChannel(Ljava/nio/file/Path;Ljava/util/Set;[Ljava/nio/file/attribute/FileAttribute;)Ljava/nio/channels/SeekableByteChannel;",
        "newBufferedReader(Ljava/nio/file/Path;)Ljava/io/BufferedReader;",
        "newBufferedReader(Ljava/nio/file/Path;Ljava/nio/charset/Charset;)Ljava/io/BufferedReader;",
        "newBufferedWriter(Ljava/nio/file/Path;Ljava/nio/charset/Charset;[Ljava/nio/file/OpenOption;)Ljava/io/BufferedWriter;",
        "readAllBytes(Ljava/nio/file/Path;)[B",
        "readString(Ljava/nio/file/Path;)Ljava/lang/String;",
        "readString(Ljava/nio/file/Path;Ljava/nio/charset/Charset;)Ljava/lang/String;",
        "readAllLines(Ljava/nio/file/Path;)Ljava/util/List;",
        "readAllLines(Ljava/nio/file/Path;Ljava/nio/charset/Charset;)Ljava/util/List;",
        "lines(Ljava/nio/file/Path;)Ljava/util/stream/Stream;",
        "lines(Ljava/nio/file/Path;Ljava/nio/charset/Charset;)Ljava/util/stream/Stream;",
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
        "copy(Ljava/nio/file/Path;Ljava/io/OutputStream;)J",
        "move(Ljava/nio/file/Path;Ljava/nio/file/Path;[Ljava/nio/file/CopyOption;)Ljava/nio/file/Path;",
        "exists(Ljava/nio/file/Path;[Ljava/nio/file/LinkOption;)Z",
        "notExists(Ljava/nio/file/Path;[Ljava/nio/file/LinkOption;)Z",
        "isDirectory(Ljava/nio/file/Path;[Ljava/nio/file/LinkOption;)Z",
        "isRegularFile(Ljava/nio/file/Path;[Ljava/nio/file/LinkOption;)Z",
        "size(Ljava/nio/file/Path;)J",
        "getLastModifiedTime(Ljava/nio/file/Path;[Ljava/nio/file/LinkOption;)Ljava/nio/file/attribute/FileTime;",
        "setLastModifiedTime(Ljava/nio/file/Path;Ljava/nio/file/attribute/FileTime;)Ljava/nio/file/Path;",
        "getOwner(Ljava/nio/file/Path;[Ljava/nio/file/LinkOption;)Ljava/nio/file/attribute/UserPrincipal;",
        "setOwner(Ljava/nio/file/Path;Ljava/nio/file/attribute/UserPrincipal;)Ljava/nio/file/Path;",
        "getAttribute(Ljava/nio/file/Path;Ljava/lang/String;[Ljava/nio/file/LinkOption;)Ljava/lang/Object;",
        "setAttribute(Ljava/nio/file/Path;Ljava/lang/String;Ljava/lang/Object;[Ljava/nio/file/LinkOption;)Ljava/nio/file/Path;",
        "list(Ljava/nio/file/Path;)Ljava/util/stream/Stream;",
        "walk(Ljava/nio/file/Path;[Ljava/nio/file/FileVisitOption;)Ljava/util/stream/Stream;",
        "walk(Ljava/nio/file/Path;I[Ljava/nio/file/FileVisitOption;)Ljava/util/stream/Stream;",
        "newDirectoryStream(Ljava/nio/file/Path;)Ljava/nio/file/DirectoryStream;",
        "newDirectoryStream(Ljava/nio/file/Path;Ljava/lang/String;)Ljava/nio/file/DirectoryStream;",
        "newDirectoryStream(Ljava/nio/file/Path;Ljava/nio/file/DirectoryStream$Filter;)Ljava/nio/file/DirectoryStream;"
    );

    private ObservedFiles() {}

    public static boolean supports(String name, String descriptor) {
        return SUPPORTED.contains(name + descriptor);
    }

    private static boolean before(Path path) {
        return Files.exists(path);
    }

    private static <T> T read(Path path, IoSupplier<T> operation) throws IOException {
        T result = operation.get();
        Recorder.read(path);
        return result;
    }

    private static <T> T write(Path path, boolean existed, IoSupplier<T> operation) throws IOException {
        T result = operation.get();
        Recorder.write(path, existed);
        return result;
    }

    public static InputStream newInputStream(Path path, OpenOption... options) throws IOException {
        return read(path, () -> Files.newInputStream(path, options));
    }

    public static OutputStream newOutputStream(Path path, OpenOption... options) throws IOException {
        boolean existed = before(path);
        return write(path, existed, () -> Files.newOutputStream(path, options));
    }

    public static SeekableByteChannel newByteChannel(Path path, OpenOption... options) throws IOException {
        boolean existed = before(path);
        SeekableByteChannel channel = Files.newByteChannel(path, options);
        observeOptions(path, existed, Set.of(options));
        return channel;
    }

    public static SeekableByteChannel newByteChannel(Path path, Set<? extends OpenOption> options, FileAttribute<?>... attributes) throws IOException {
        boolean existed = before(path);
        SeekableByteChannel channel = Files.newByteChannel(path, options, attributes);
        observeOptions(path, existed, options);
        return channel;
    }

    public static BufferedReader newBufferedReader(Path path) throws IOException {
        return read(path, () -> Files.newBufferedReader(path));
    }

    public static BufferedReader newBufferedReader(Path path, Charset charset) throws IOException {
        return read(path, () -> Files.newBufferedReader(path, charset));
    }

    public static BufferedWriter newBufferedWriter(Path path, Charset charset, OpenOption... options) throws IOException {
        boolean existed = before(path);
        return write(path, existed, () -> Files.newBufferedWriter(path, charset, options));
    }

    public static byte[] readAllBytes(Path path) throws IOException { return read(path, () -> Files.readAllBytes(path)); }
    public static String readString(Path path) throws IOException { return read(path, () -> Files.readString(path)); }
    public static String readString(Path path, Charset charset) throws IOException { return read(path, () -> Files.readString(path, charset)); }
    public static List<String> readAllLines(Path path) throws IOException { return read(path, () -> Files.readAllLines(path)); }
    public static List<String> readAllLines(Path path, Charset charset) throws IOException { return read(path, () -> Files.readAllLines(path, charset)); }
    public static Stream<String> lines(Path path) throws IOException { return read(path, () -> Files.lines(path)); }
    public static Stream<String> lines(Path path, Charset charset) throws IOException { return read(path, () -> Files.lines(path, charset)); }

    public static Path write(Path path, byte[] bytes, OpenOption... options) throws IOException {
        boolean existed = before(path);
        return write(path, existed, () -> Files.write(path, bytes, options));
    }

    public static Path write(Path path, Iterable<? extends CharSequence> lines, OpenOption... options) throws IOException {
        boolean existed = before(path);
        return write(path, existed, () -> Files.write(path, lines, options));
    }

    public static Path write(Path path, Iterable<? extends CharSequence> lines, Charset charset, OpenOption... options) throws IOException {
        boolean existed = before(path);
        return write(path, existed, () -> Files.write(path, lines, charset, options));
    }

    public static Path writeString(Path path, CharSequence value, OpenOption... options) throws IOException {
        boolean existed = before(path);
        return write(path, existed, () -> Files.writeString(path, value, options));
    }

    public static Path writeString(Path path, CharSequence value, Charset charset, OpenOption... options) throws IOException {
        boolean existed = before(path);
        return write(path, existed, () -> Files.writeString(path, value, charset, options));
    }

    public static Path createFile(Path path, FileAttribute<?>... attributes) throws IOException {
        Path result = Files.createFile(path, attributes);
        Recorder.write(path, false);
        return result;
    }

    public static Path createDirectory(Path path, FileAttribute<?>... attributes) throws IOException {
        Path result = Files.createDirectory(path, attributes);
        Recorder.tree(path, false, true);
        return result;
    }

    public static Path createDirectories(Path path, FileAttribute<?>... attributes) throws IOException {
        Path createdRoot = Recorder.firstOwnedMissingAncestor(path);
        Path result = Files.createDirectories(path, attributes);
        if (createdRoot != null) Recorder.tree(createdRoot, false, true);
        return result;
    }

    public static Path createTempFile(Path directory, String prefix, String suffix, FileAttribute<?>... attributes) throws IOException {
        Path result = Files.createTempFile(directory, prefix, suffix, attributes);
        Recorder.write(result, false);
        return result;
    }

    public static Path createTempFile(String prefix, String suffix, FileAttribute<?>... attributes) throws IOException {
        Path result = Files.createTempFile(prefix, suffix, attributes);
        Recorder.write(result, false);
        return result;
    }

    public static Path createTempDirectory(Path directory, String prefix, FileAttribute<?>... attributes) throws IOException {
        Path result = Files.createTempDirectory(directory, prefix, attributes);
        Recorder.tree(result, false, true);
        return result;
    }

    public static Path createTempDirectory(String prefix, FileAttribute<?>... attributes) throws IOException {
        Path result = Files.createTempDirectory(prefix, attributes);
        Recorder.tree(result, false, true);
        return result;
    }

    public static void delete(Path path) throws IOException { Files.delete(path); Recorder.write(path, true); }
    public static boolean deleteIfExists(Path path) throws IOException {
        boolean deleted = Files.deleteIfExists(path);
        if (deleted) Recorder.write(path, true);
        return deleted;
    }

    public static Path copy(Path source, Path target, CopyOption... options) throws IOException {
        boolean existed = before(target);
        Path result = Files.copy(source, target, options);
        Recorder.read(source);
        Recorder.write(target, existed);
        return result;
    }

    public static long copy(InputStream source, Path target, CopyOption... options) throws IOException {
        boolean existed = before(target);
        long result = Files.copy(source, target, options);
        Recorder.write(target, existed);
        return result;
    }

    public static long copy(Path source, OutputStream target) throws IOException {
        long result = Files.copy(source, target);
        Recorder.read(source);
        return result;
    }

    public static Path move(Path source, Path target, CopyOption... options) throws IOException {
        boolean existed = before(target);
        Path result = Files.move(source, target, options);
        Recorder.write(source, true);
        Recorder.write(target, existed);
        return result;
    }

    public static boolean exists(Path path, LinkOption... options) { Recorder.read(path); return Files.exists(path, options); }
    public static boolean notExists(Path path, LinkOption... options) { Recorder.read(path); return Files.notExists(path, options); }
    public static boolean isDirectory(Path path, LinkOption... options) { Recorder.read(path); return Files.isDirectory(path, options); }
    public static boolean isRegularFile(Path path, LinkOption... options) { Recorder.read(path); return Files.isRegularFile(path, options); }
    public static long size(Path path) throws IOException { return read(path, () -> Files.size(path)); }
    public static FileTime getLastModifiedTime(Path path, LinkOption... options) throws IOException { return read(path, () -> Files.getLastModifiedTime(path, options)); }
    public static Path setLastModifiedTime(Path path, FileTime value) throws IOException { return write(path, true, () -> Files.setLastModifiedTime(path, value)); }
    public static UserPrincipal getOwner(Path path, LinkOption... options) throws IOException { return read(path, () -> Files.getOwner(path, options)); }
    public static Path setOwner(Path path, UserPrincipal owner) throws IOException { return write(path, true, () -> Files.setOwner(path, owner)); }
    public static Object getAttribute(Path path, String attribute, LinkOption... options) throws IOException { return read(path, () -> Files.getAttribute(path, attribute, options)); }
    public static Path setAttribute(Path path, String attribute, Object value, LinkOption... options) throws IOException { return write(path, true, () -> Files.setAttribute(path, attribute, value, options)); }
    public static Stream<Path> list(Path path) throws IOException { return read(path, () -> Files.list(path)); }
    public static Stream<Path> walk(Path path, FileVisitOption... options) throws IOException { return read(path, () -> Files.walk(path, options)); }
    public static Stream<Path> walk(Path path, int depth, FileVisitOption... options) throws IOException { return read(path, () -> Files.walk(path, depth, options)); }
    public static DirectoryStream<Path> newDirectoryStream(Path path) throws IOException { return read(path, () -> Files.newDirectoryStream(path)); }
    public static DirectoryStream<Path> newDirectoryStream(Path path, String glob) throws IOException { return read(path, () -> Files.newDirectoryStream(path, glob)); }
    public static DirectoryStream<Path> newDirectoryStream(Path path, DirectoryStream.Filter<? super Path> filter) throws IOException { return read(path, () -> Files.newDirectoryStream(path, filter)); }

    public static boolean fileCreateNewFile(java.io.File file) throws IOException {
        boolean created = file.createNewFile();
        if (created) Recorder.write(file.toPath(), false);
        return created;
    }

    public static boolean fileDelete(java.io.File file) {
        boolean deleted = file.delete();
        if (deleted) Recorder.write(file.toPath(), true);
        return deleted;
    }

    public static boolean fileMkdir(java.io.File file) {
        boolean created = file.mkdir();
        if (created) Recorder.tree(file.toPath(), false, true);
        return created;
    }

    public static boolean fileMkdirs(java.io.File file) {
        Path createdRoot = Recorder.firstOwnedMissingAncestor(file.toPath());
        boolean created = file.mkdirs();
        if (created && createdRoot != null) Recorder.tree(createdRoot, false, true);
        return created;
    }

    public static boolean fileRenameTo(java.io.File source, java.io.File target) {
        boolean targetExisted = target.exists();
        boolean moved = source.renameTo(target);
        if (moved) {
            Recorder.write(source.toPath(), true);
            Recorder.write(target.toPath(), targetExisted);
        }
        return moved;
    }

    private static void observeOptions(Path path, boolean existed, Set<? extends OpenOption> options) {
        boolean write = options.contains(java.nio.file.StandardOpenOption.WRITE)
            || options.contains(java.nio.file.StandardOpenOption.APPEND)
            || options.contains(java.nio.file.StandardOpenOption.CREATE)
            || options.contains(java.nio.file.StandardOpenOption.CREATE_NEW)
            || options.contains(java.nio.file.StandardOpenOption.DELETE_ON_CLOSE);
        if (write) Recorder.write(path, existed); else Recorder.read(path);
    }

    @FunctionalInterface
    private interface IoSupplier<T> { T get() throws IOException; }
}
