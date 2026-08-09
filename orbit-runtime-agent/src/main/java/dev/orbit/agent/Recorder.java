package dev.orbit.agent;

import java.io.BufferedWriter;
import java.net.URI;
import java.net.URL;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.StandardCopyOption;
import java.security.MessageDigest;
import java.util.ArrayDeque;
import java.util.ArrayList;
import java.util.Base64;
import java.util.Comparator;
import java.util.HexFormat;
import java.util.List;
import java.util.Map;
import java.util.Set;
import java.util.concurrent.ConcurrentHashMap;
import java.util.concurrent.ConcurrentSkipListMap;
import java.util.concurrent.atomic.AtomicBoolean;

/** Low-allocation path ownership recorder called only at file-operation boundaries. */
final class Recorder {
    private static final String[] SHARED_ROOTS = {
        "assets", "config", "libraries", "logs", "mods", "natives", "resourcepacks",
        "saves", "screenshots", "shaderpacks", "versions"
    };
    private static final StackWalker WALKER = StackWalker.getInstance(StackWalker.Option.RETAIN_CLASS_REFERENCE);
    private static final Map<Class<?>, String> CLASS_OWNERS = new ConcurrentHashMap<>();
    private static final Map<Path, String> JAR_OWNERS = new ConcurrentHashMap<>();
    private static final ConcurrentSkipListMap<String, Record> RECORDS = new ConcurrentSkipListMap<>();
    private static final ConcurrentSkipListMap<String, Record> TREES = new ConcurrentSkipListMap<>();
    private static final ThreadLocal<ArrayDeque<Boolean>> BEFORE = ThreadLocal.withInitial(ArrayDeque::new);
    private static final AtomicBoolean DIRTY = new AtomicBoolean();
    private static final Object FLUSH_LOCK = new Object();

    private static volatile Path instanceRoot;
    private static volatile Path modsRoot;
    private static volatile Path sessionFile;

    private Recorder() {}

    static void configure(Path root, Path session) throws Exception {
        instanceRoot = root.toAbsolutePath().normalize();
        modsRoot = instanceRoot.resolve("mods").normalize();
        sessionFile = session.toAbsolutePath().normalize();
        Files.createDirectories(sessionFile.getParent());
        Thread writer = new Thread(() -> {
            while (true) {
                try {
                    Thread.sleep(1000L);
                    flushIfDirty();
                } catch (InterruptedException interrupted) {
                    Thread.currentThread().interrupt();
                    return;
                } catch (Throwable ignored) {
                    // A later tick or shutdown hook retries the complete snapshot.
                }
            }
        }, "orbit-data-recorder");
        writer.setDaemon(true);
        writer.start();
        Runtime.getRuntime().addShutdownHook(new Thread(() -> {
            try {
                flush();
            } catch (Throwable ignored) {
                // Launcher also recovers the latest periodic snapshot after crashes.
            }
        }, "orbit-data-recorder-shutdown"));
    }

    static String beforePath(String path) {
        Path normalized = normalize(path);
        BEFORE.get().push(normalized != null && Files.exists(normalized));
        return path;
    }

    static java.io.File beforeFile(java.io.File file) {
        Path normalized = normalize(file.toPath());
        BEFORE.get().push(normalized != null && Files.exists(normalized));
        return file;
    }

    static boolean takeBefore() {
        ArrayDeque<Boolean> values = BEFORE.get();
        return values.isEmpty() || values.pop();
    }

    static void read(Path path) {
        record(path, false, true, false, false);
    }

    static void write(Path path, boolean existedBefore) {
        record(path, false, false, true, !existedBefore);
    }

    static void tree(Path path, boolean existedBefore, boolean write) {
        record(path, true, !write, write, !existedBefore);
    }

    static Path firstOwnedMissingAncestor(Path rawPath) {
        Path path = normalize(rawPath);
        Path missing = null;
        while (path != null && !Files.exists(path)) {
            if (!isSharedRoot(path) && !path.equals(instanceRoot)) {
                missing = path;
            }
            path = path.getParent();
        }
        return missing;
    }

    private static void record(Path rawPath, boolean tree, boolean read, boolean write, boolean created) {
        Path path = normalize(rawPath);
        if (path == null || path.equals(instanceRoot) || isControlPath(path)) {
            return;
        }
        String owner = currentOwner();
        if (owner == null) {
            return;
        }
        if (tree && created && isSharedRoot(path)) {
            tree = false;
        }
        String key = key(path, tree);
        boolean recordTree = tree;
        if (!tree) {
            Map.Entry<String, Record> floor = TREES.floorEntry(normalizedString(path));
            if (floor != null && path.startsWith(floor.getValue().path)) {
                floor.getValue().merge(owner, read, write, created);
                DIRTY.set(true);
                return;
            }
        }
        Record record = RECORDS.computeIfAbsent(key, ignored -> new Record(path, recordTree));
        if (tree) {
            TREES.putIfAbsent(normalizedString(path), record);
        }
        record.merge(owner, read, write, created);
        if (tree && created) {
            compactChildren(record);
        }
        DIRTY.set(true);
    }

    private static void compactChildren(Record tree) {
        String prefix = normalizedString(tree.path);
        List<String> remove = new ArrayList<>();
        for (Map.Entry<String, Record> entry : RECORDS.tailMap(prefix, false).entrySet()) {
            Record child = entry.getValue();
            if (child == tree) {
                continue;
            }
            if (!child.path.startsWith(tree.path)) {
                break;
            }
            tree.readers.addAll(child.readers);
            tree.writers.addAll(child.writers);
            tree.creators.addAll(child.creators);
            remove.add(entry.getKey());
        }
        for (String key : remove) {
            Record removed = RECORDS.remove(key);
            if (removed != null && removed.tree) {
                TREES.remove(normalizedString(removed.path), removed);
            }
        }
    }

    private static String currentOwner() {
        return WALKER.walk(frames -> frames
            .map(StackWalker.StackFrame::getDeclaringClass)
            .map(Recorder::ownerForClass)
            .filter(owner -> owner != null)
            .findFirst()
            .orElse(null));
    }

    private static String ownerForClass(Class<?> type) {
        String owner = CLASS_OWNERS.computeIfAbsent(type, key -> {
            try {
                if (key.getProtectionDomain() == null
                    || key.getProtectionDomain().getCodeSource() == null) {
                    return "";
                }
                URL location = key.getProtectionDomain().getCodeSource().getLocation();
                Path jar = codeSourcePath(location);
                if (jar == null || !jar.startsWith(modsRoot) || !Files.isRegularFile(jar)) {
                    return "";
                }
                return JAR_OWNERS.computeIfAbsent(jar, Recorder::sha256Unchecked);
            } catch (Throwable ignored) {
                return "";
            }
        });
        return owner.isEmpty() ? null : owner;
    }

    private static Path codeSourcePath(URL location) throws Exception {
        String external = location.toExternalForm();
        int nested = external.indexOf("!/");
        if (nested >= 0) {
            external = external.substring(0, nested);
        }
        while (external.startsWith("jar:")) {
            external = external.substring(4);
        }
        URI uri = URI.create(external);
        if (!"file".equalsIgnoreCase(uri.getScheme())) {
            return null;
        }
        return Path.of(uri).toAbsolutePath().normalize();
    }

    private static String sha256Unchecked(Path path) {
        try {
            MessageDigest digest = MessageDigest.getInstance("SHA-256");
            try (var input = Files.newInputStream(path)) {
                byte[] buffer = new byte[128 * 1024];
                int read;
                while ((read = input.read(buffer)) >= 0) {
                    digest.update(buffer, 0, read);
                }
            }
            return HexFormat.of().formatHex(digest.digest());
        } catch (Throwable ignored) {
            return "";
        }
    }

    private static Path normalize(String path) {
        return path == null ? null : normalize(Path.of(path));
    }

    private static Path normalize(Path path) {
        if (path == null) {
            return null;
        }
        try {
            return path.toAbsolutePath().normalize();
        } catch (Throwable ignored) {
            return null;
        }
    }

    private static boolean isSharedRoot(Path path) {
        if (!path.startsWith(instanceRoot)) {
            return false;
        }
        Path relative = instanceRoot.relativize(path);
        if (relative.getNameCount() != 1) {
            return false;
        }
        String name = relative.getFileName().toString();
        for (String shared : SHARED_ROOTS) {
            if (name.equals(shared)) {
                return true;
            }
        }
        return false;
    }

    private static boolean isControlPath(Path path) {
        return path.startsWith(instanceRoot.resolve(".orbit").normalize());
    }

    private static String key(Path path, boolean tree) {
        return normalizedString(path) + (tree ? "\u0000tree" : "\u0000file");
    }

    private static String normalizedString(Path path) {
        String value = path.toString();
        return System.getProperty("os.name", "").toLowerCase().contains("win")
            ? value.toLowerCase()
            : value;
    }

    private static void flushIfDirty() throws Exception {
        if (DIRTY.compareAndSet(true, false)) {
            flush();
        }
    }

    private static void flush() throws Exception {
        synchronized (FLUSH_LOCK) {
            Path temporary = sessionFile.resolveSibling(sessionFile.getFileName() + ".tmp");
            List<Record> snapshot = RECORDS.values().stream()
                .sorted(Comparator.comparing(record -> record.path.toString()))
                .toList();
            try (BufferedWriter writer = Files.newBufferedWriter(temporary, StandardCharsets.UTF_8)) {
                for (Record record : snapshot) {
                    Set<String> owners = new java.util.TreeSet<>();
                    owners.addAll(record.creators);
                    owners.addAll(record.readers);
                    owners.addAll(record.writers);
                    for (String owner : owners) {
                        writer.write("1\t");
                        writer.write(record.tree ? "tree" : "file");
                        writer.write('\t');
                        writer.write(record.creators.contains(owner) ? '1' : '0');
                        writer.write(record.readers.contains(owner) ? '1' : '0');
                        writer.write(record.writers.contains(owner) ? '1' : '0');
                        writer.write('\t');
                        writer.write(owner);
                        writer.write('\t');
                        writer.write(Base64.getUrlEncoder().withoutPadding().encodeToString(
                            record.path.toString().getBytes(StandardCharsets.UTF_8)
                        ));
                        writer.write("\tend\n");
                    }
                }
            }
            try {
                Files.move(temporary, sessionFile, StandardCopyOption.ATOMIC_MOVE, StandardCopyOption.REPLACE_EXISTING);
            } catch (java.nio.file.AtomicMoveNotSupportedException ignored) {
                Files.move(temporary, sessionFile, StandardCopyOption.REPLACE_EXISTING);
            }
        }
    }

    private static final class Record {
        private final Path path;
        private final boolean tree;
        private final Set<String> creators = ConcurrentHashMap.newKeySet();
        private final Set<String> readers = ConcurrentHashMap.newKeySet();
        private final Set<String> writers = ConcurrentHashMap.newKeySet();

        private Record(Path path, boolean tree) {
            this.path = path;
            this.tree = tree;
        }

        private void merge(String owner, boolean read, boolean write, boolean created) {
            if (created) creators.add(owner);
            if (read) readers.add(owner);
            if (write) writers.add(owner);
        }
    }
}
