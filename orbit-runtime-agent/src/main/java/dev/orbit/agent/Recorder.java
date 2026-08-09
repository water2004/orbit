package dev.orbit.agent;

import java.io.BufferedWriter;
import java.net.URI;
import java.net.URL;
import java.nio.charset.StandardCharsets;
import java.nio.file.FileSystems;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.StandardCopyOption;
import java.security.MessageDigest;
import java.security.ProtectionDomain;
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
import java.util.concurrent.atomic.AtomicLong;
import java.util.function.Function;

/** Low-allocation mutation ownership recorder called only from managed package classes. */
public final class Recorder {
    private static final Map<String, String> SOURCE_OWNERS = new ConcurrentHashMap<>();
    private static final Map<Path, String> PATH_OWNERS = new ConcurrentHashMap<>();
    private static final ConcurrentSkipListMap<String, State> STATES = new ConcurrentSkipListMap<>();
    private static final ConcurrentSkipListMap<String, OwnedTree> OWNED_TREES = new ConcurrentSkipListMap<>();
    private static final ConcurrentSkipListMap<String, Path> EXPLICIT_NODES = new ConcurrentSkipListMap<>();
    private static final ConcurrentSkipListMap<String, Path> PUBLISHED_NODES = new ConcurrentSkipListMap<>();
    private static final Set<String> RESERVED = ConcurrentHashMap.newKeySet();
    private static final ThreadLocal<ArrayDeque<Boolean>> BEFORE = ThreadLocal.withInitial(ArrayDeque::new);
    private static final AtomicLong MUTATION_VERSION = new AtomicLong();
    private static final Object FLUSH_LOCK = new Object();

    private static volatile Path instanceRoot;
    private static volatile Path sessionFile;
    private static volatile long flushedVersion;

    private Recorder() {}

    public static void configure(Path root, Path session, Path context) throws Exception {
        instanceRoot = root.toAbsolutePath().normalize();
        sessionFile = session.toAbsolutePath().normalize();
        loadContext(context.toAbsolutePath().normalize());
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

    /** Resolve one class definition to the selected top-level package artifact. */
    public static String ownerFor(ProtectionDomain domain) {
        try {
            if (domain == null || domain.getCodeSource() == null) {
                return null;
            }
            Path source = codeSourcePath(domain.getCodeSource().getLocation());
            if (source == null || !Files.isRegularFile(source)) {
                return null;
            }
            String owner = PATH_OWNERS.computeIfAbsent(source, path -> {
                String digest = sha256Unchecked(path);
                return digest.isEmpty() ? "" : SOURCE_OWNERS.getOrDefault(digest, "");
            });
            return owner.isEmpty() ? null : owner;
        } catch (Throwable ignored) {
            return null;
        }
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

    static void write(Path path, boolean existedBefore, String owner) {
        if (existedBefore) {
            mutate(path, false, owner);
        } else {
            create(path, false, owner);
        }
    }

    static void tree(Path path, boolean existedBefore, boolean write, String owner) {
        if (!write) {
            return;
        }
        if (existedBefore) {
            mutate(path, true, owner);
        } else {
            create(path, true, owner);
        }
    }

    static void delete(Path path, boolean tree, String owner) {
        Path normalized = normalize(path);
        if (!validMutation(normalized, owner)) {
            return;
        }
        synchronized (FLUSH_LOCK) {
            String key = normalizedString(normalized);
            State exact = STATES.remove(key);
            boolean removedState = exact != null;
            removedState |= removeDescendants(STATES, normalized, key, state -> state.path);
            OWNED_TREES.remove(key);
            removeDescendants(OWNED_TREES, normalized, key, treeOwner -> treeOwner.path);
            boolean mustPublish = nodeAtOrBelow(PUBLISHED_NODES, normalized, key, tree)
                || nodeAtOrBelow(EXPLICIT_NODES, normalized, key, tree);
            if (!mustPublish) {
                if (removedState) {
                    markDirty();
                }
                return;
            }
            State state = new State(normalized, tree);
            state.deletedBy = owner;
            STATES.put(key, state);
            markDirty();
        }
    }

    static Path firstOwnedMissingAncestor(Path rawPath) {
        Path path = normalize(rawPath);
        Path missing = null;
        while (path != null && !Files.exists(path)) {
            if (!isReserved(path) && !path.equals(instanceRoot)) {
                missing = path;
            }
            path = path.getParent();
        }
        return missing;
    }

    private static void create(Path rawPath, boolean tree, String owner) {
        Path path = normalize(rawPath);
        if (!validMutation(path, owner) || (tree && isReserved(path))) {
            return;
        }
        synchronized (FLUSH_LOCK) {
            OwnedTree inherited = nearestTree(path);
            if (inherited != null && inherited.owner.equals(owner)) {
                if (STATES.remove(normalizedString(path)) != null) {
                    markDirty();
                }
                return;
            }
            String key = normalizedString(path);
            State state = new State(path, tree);
            state.creator = owner;
            STATES.put(key, state);
            if (tree) {
                OWNED_TREES.put(key, new OwnedTree(path, owner));
                compactSameOwnerDescendants(path, key, owner);
            }
            markDirty();
        }
    }

    private static void mutate(Path rawPath, boolean tree, String owner) {
        Path path = normalize(rawPath);
        if (!validMutation(path, owner)) {
            return;
        }
        synchronized (FLUSH_LOCK) {
            String key = normalizedString(path);
            State exact = STATES.get(key);
            if (exact != null && !exact.deleted() && owner.equals(exact.creator)) {
                return;
            }
            OwnedTree inherited = nearestTree(path);
            if (inherited != null && inherited.owner.equals(owner)) {
                return;
            }
            State state = STATES.computeIfAbsent(key, ignored -> new State(path, tree));
            if (!state.deleted()) {
                state.writers.add(owner);
                markDirty();
            }
        }
    }

    private static boolean validMutation(Path path, String owner) {
        return path != null
            && owner != null
            && !owner.isBlank()
            && !path.equals(instanceRoot)
            && !isControlPath(path);
    }

    private static OwnedTree nearestTree(Path path) {
        Map.Entry<String, OwnedTree> entry = OWNED_TREES.floorEntry(normalizedString(path));
        while (entry != null) {
            OwnedTree tree = entry.getValue();
            if (!tree.path.equals(path) && path.startsWith(tree.path)) {
                return tree;
            }
            entry = OWNED_TREES.lowerEntry(entry.getKey());
        }
        return null;
    }

    private static <T> boolean removeDescendants(
        ConcurrentSkipListMap<String, T> map,
        Path parent,
        String parentKey,
        Function<T, Path> pathOf
    ) {
        List<String> remove = new ArrayList<>();
        for (Map.Entry<String, T> entry : map.tailMap(parentKey, false).entrySet()) {
            Path child = pathOf.apply(entry.getValue());
            if (!child.startsWith(parent)) {
                break;
            }
            remove.add(entry.getKey());
        }
        remove.forEach(map::remove);
        return !remove.isEmpty();
    }

    private static boolean nodeAtOrBelow(
        ConcurrentSkipListMap<String, Path> nodes,
        Path path,
        String key,
        boolean tree
    ) {
        if (nodes.containsKey(key)) {
            return true;
        }
        if (!tree) {
            return false;
        }
        for (Path candidate : nodes.tailMap(key, false).values()) {
            if (!candidate.startsWith(path)) {
                break;
            }
            return true;
        }
        return false;
    }

    private static void compactSameOwnerDescendants(Path parent, String parentKey, String owner) {
        List<String> remove = new ArrayList<>();
        for (Map.Entry<String, State> entry : STATES.tailMap(parentKey, false).entrySet()) {
            State child = entry.getValue();
            if (!child.path.startsWith(parent)) {
                break;
            }
            if (!child.deleted()
                && (child.creator == null || owner.equals(child.creator))
                && (child.writers.isEmpty() || child.writers.equals(Set.of(owner)))) {
                remove.add(entry.getKey());
            }
        }
        remove.forEach(STATES::remove);
    }

    private static void loadContext(Path context) throws Exception {
        List<String> lines = Files.readAllLines(context, StandardCharsets.UTF_8);
        if (lines.isEmpty() || !lines.get(0).equals("2\tcontext\tend")) {
            throw new IllegalArgumentException("invalid runtime ownership context header");
        }
        for (int index = 1; index < lines.size(); index++) {
            String[] fields = lines.get(index).split("\t", -1);
            if (fields.length == 4 && fields[0].equals("source") && fields[3].equals("end")) {
                requireDigest(fields[1]);
                requireDigest(fields[2]);
                String previous = SOURCE_OWNERS.putIfAbsent(fields[1], fields[2]);
                if (previous != null && !previous.equals(fields[2])) {
                    SOURCE_OWNERS.remove(fields[1]);
                }
            } else if (fields.length == 5 && fields[0].equals("node") && fields[4].equals("end")) {
                if (!fields[1].equals("file") && !fields[1].equals("tree")) {
                    throw new IllegalArgumentException("invalid runtime ownership node kind");
                }
                if (!fields[2].equals("-")) {
                    requireDigest(fields[2]);
                }
                Path path = decodePath(fields[3]);
                String key = normalizedString(path);
                EXPLICIT_NODES.put(key, path);
                if (fields[1].equals("tree") && !fields[2].equals("-")) {
                    OWNED_TREES.put(key, new OwnedTree(path, fields[2]));
                }
            } else if (fields.length == 3 && fields[0].equals("reserved") && fields[2].equals("end")) {
                RESERVED.add(normalizedString(decodePath(fields[1])));
            } else {
                throw new IllegalArgumentException("invalid runtime ownership context record at line " + (index + 1));
            }
        }
    }

    private static void requireDigest(String value) {
        if (value.length() != 64 || !value.chars().allMatch(character -> Character.digit(character, 16) >= 0)) {
            throw new IllegalArgumentException("invalid SHA-256 in runtime ownership context");
        }
    }

    private static Path decodePath(String value) {
        Path path = Path.of(new String(Base64.getUrlDecoder().decode(value), StandardCharsets.UTF_8));
        Path normalized = normalize(path);
        if (normalized == null) {
            throw new IllegalArgumentException("runtime ownership context path is not physical");
        }
        return normalized;
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
            if (path.getFileSystem() != FileSystems.getDefault()) {
                return null;
            }
            return path.toAbsolutePath().normalize();
        } catch (Throwable ignored) {
            return null;
        }
    }

    private static boolean isReserved(Path path) {
        return RESERVED.contains(normalizedString(path));
    }

    private static boolean isControlPath(Path path) {
        return path.startsWith(instanceRoot.resolve(".orbit").normalize());
    }

    private static String normalizedString(Path path) {
        String value = path.toString();
        return System.getProperty("os.name", "").toLowerCase().contains("win")
            ? value.toLowerCase()
            : value;
    }

    private static void markDirty() {
        MUTATION_VERSION.incrementAndGet();
    }

    private static void flushIfDirty() throws Exception {
        long targetVersion = MUTATION_VERSION.get();
        if (targetVersion != flushedVersion) {
            flush();
            flushedVersion = targetVersion;
        }
    }

    private static void flush() throws Exception {
        synchronized (FLUSH_LOCK) {
            Path temporary = sessionFile.resolveSibling(sessionFile.getFileName() + ".tmp");
            List<State> snapshot = STATES.values().stream()
                .sorted(Comparator.comparing(state -> state.path.toString()))
                .toList();
            try (BufferedWriter writer = Files.newBufferedWriter(temporary, StandardCharsets.UTF_8)) {
                for (State state : snapshot) {
                    if (state.deleted()) {
                        writeRecord(writer, "delete", state, state.deletedBy);
                        continue;
                    }
                    if (state.creator != null) {
                        writeRecord(writer, "create", state, state.creator);
                    }
                    for (String owner : new java.util.TreeSet<>(state.writers)) {
                        if (!owner.equals(state.creator)) {
                            writeRecord(writer, "write", state, owner);
                        }
                    }
                }
            }
            try {
                Files.move(temporary, sessionFile, StandardCopyOption.ATOMIC_MOVE, StandardCopyOption.REPLACE_EXISTING);
            } catch (java.nio.file.AtomicMoveNotSupportedException ignored) {
                Files.move(temporary, sessionFile, StandardCopyOption.REPLACE_EXISTING);
            }
            for (State state : snapshot) {
                PUBLISHED_NODES.put(normalizedString(state.path), state.path);
            }
        }
    }

    private static void writeRecord(BufferedWriter writer, String action, State state, String owner) throws Exception {
        writer.write("2\t");
        writer.write(action);
        writer.write('\t');
        writer.write(state.tree ? "tree" : "file");
        writer.write('\t');
        writer.write(owner);
        writer.write('\t');
        writer.write(Base64.getUrlEncoder().withoutPadding().encodeToString(
            state.path.toString().getBytes(StandardCharsets.UTF_8)
        ));
        writer.write("\tend\n");
    }

    private static final class State {
        private final Path path;
        private final boolean tree;
        private final Set<String> writers = ConcurrentHashMap.newKeySet();
        private volatile String creator;
        private volatile String deletedBy;

        private State(Path path, boolean tree) {
            this.path = path;
            this.tree = tree;
        }

        private boolean deleted() {
            return deletedBy != null;
        }
    }

    private record OwnedTree(Path path, String owner) {}
}
