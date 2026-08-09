package dev.orbit.agent;

import java.io.IOException;
import java.nio.channels.AsynchronousFileChannel;
import java.nio.channels.FileChannel;
import java.nio.file.OpenOption;
import java.nio.file.Path;
import java.nio.file.StandardOpenOption;
import java.nio.file.attribute.FileAttribute;
import java.util.Set;
import java.util.concurrent.ExecutorService;

public final class ObservedChannels {
    private ObservedChannels() {}

    static boolean supports(String owner, String descriptor) {
        return (owner.equals("java/nio/channels/FileChannel")
                && (descriptor.equals("(Ljava/nio/file/Path;[Ljava/nio/file/OpenOption;)Ljava/nio/channels/FileChannel;")
                    || descriptor.equals("(Ljava/nio/file/Path;Ljava/util/Set;[Ljava/nio/file/attribute/FileAttribute;)Ljava/nio/channels/FileChannel;")))
            || (owner.equals("java/nio/channels/AsynchronousFileChannel")
                && (descriptor.equals("(Ljava/nio/file/Path;[Ljava/nio/file/OpenOption;)Ljava/nio/channels/AsynchronousFileChannel;")
                    || descriptor.equals("(Ljava/nio/file/Path;Ljava/util/Set;Ljava/util/concurrent/ExecutorService;[Ljava/nio/file/attribute/FileAttribute;)Ljava/nio/channels/AsynchronousFileChannel;")));
    }

    public static FileChannel fileOpen(Path path, OpenOption... options) throws IOException {
        boolean existed = java.nio.file.Files.exists(path);
        FileChannel channel = FileChannel.open(path, options);
        observe(path, existed, Set.of(options));
        return channel;
    }

    public static FileChannel fileOpen(Path path, Set<? extends OpenOption> options, FileAttribute<?>... attributes) throws IOException {
        boolean existed = java.nio.file.Files.exists(path);
        FileChannel channel = FileChannel.open(path, options, attributes);
        observe(path, existed, options);
        return channel;
    }

    public static AsynchronousFileChannel asyncOpen(Path path, OpenOption... options) throws IOException {
        boolean existed = java.nio.file.Files.exists(path);
        AsynchronousFileChannel channel = AsynchronousFileChannel.open(path, options);
        observe(path, existed, Set.of(options));
        return channel;
    }

    public static AsynchronousFileChannel asyncOpen(Path path, Set<? extends OpenOption> options, ExecutorService executor, FileAttribute<?>... attributes) throws IOException {
        boolean existed = java.nio.file.Files.exists(path);
        AsynchronousFileChannel channel = AsynchronousFileChannel.open(path, options, executor, attributes);
        observe(path, existed, options);
        return channel;
    }

    private static void observe(Path path, boolean existed, Set<? extends OpenOption> options) {
        boolean write = options.contains(StandardOpenOption.WRITE)
            || options.contains(StandardOpenOption.APPEND)
            || options.contains(StandardOpenOption.CREATE)
            || options.contains(StandardOpenOption.CREATE_NEW)
            || options.contains(StandardOpenOption.DELETE_ON_CLOSE);
        if (write) Recorder.write(path, existed); else Recorder.read(path);
    }
}
