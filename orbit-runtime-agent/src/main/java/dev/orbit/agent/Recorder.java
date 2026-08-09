package dev.orbit.agent;

import java.io.BufferedWriter;
import java.net.URI;
import java.net.URL;
import java.nio.charset.StandardCharsets;
import java.nio.channels.FileChannel;
import java.nio.channels.FileLock;
import java.nio.file.FileSystems;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.StandardCopyOption;
import java.nio.file.StandardOpenOption;
import java.security.MessageDigest;
import java.security.ProtectionDomain;
import java.util.ArrayDeque;
import java.util.ArrayList;
import java.util.Base64;
import java.util.Collections;
import java.util.Comparator;
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
    private static final Map<String, String> MODULE_OWNERS = new ConcurrentHashMap<>();
    private static final Map<String, String> SOURCE_CACHE = new ConcurrentHashMap<>();
    private static final ConcurrentSkipListMap<String, State> STATES = new ConcurrentSkipListMap<>();
    private static final ConcurrentSkipListMap<String, OwnedTree> OWNED_TREES = new ConcurrentSkipListMap<>();
    private static final ConcurrentSkipListMap<String, Path> EXPLICIT_NODES = new ConcurrentSkipListMap<>();
    private static final ConcurrentSkipListMap<String, Path> PUBLISHED_NODES = new ConcurrentSkipListMap<>();
    private static final Set<String> RESERVED = ConcurrentHashMap.newKeySet();
    private static final ThreadLocal<ArrayDeque<Boolean>> BEFORE = ThreadLocal.withInitial(ArrayDeque::new);
    private static final AtomicLong MUTATION_VERSION = new AtomicLong();
    private static final Object FLUSH_LOCK = new Object();
    private static final boolean WINDOWS = System.getProperty("os.name", "")
        .toLowerCase()
        .contains("win");

    private static volatile Path instanceRoot;
    private static volatile Path sessionFile;
    private static volatile FileChannel observationLockChannel;
    private static volatile FileLock observationLock;
    private static volatile Path observationActiveMarker;
    private static volatile String sessionId;
    private static volatile long sessionStartedAtMillis;
    private static volatile long flushedVersion;
    private static volatile boolean fileCodeSources;
    private static volatile boolean unionCodeSources;
    private static volatile boolean quiltModuleIdentity;

    private Recorder() {}

    public static void configure(Path root, Path session, Path context) throws Exception {
        instanceRoot = root.toAbsolutePath().normalize();
        acquireObservationLock();
        sessionStartedAtMillis = System.currentTimeMillis();
        sessionId = Long.toUnsignedString(sessionStartedAtMillis, 36)
            + "-" + Long.toUnsignedString(System.nanoTime(), 36);
        sessionFile = allocateSessionFile(session.toAbsolutePath().normalize());
        loadContext(context.toAbsolutePath().normalize());
        Files.createDirectories(sessionFile.getParent());
        publishObservationActivity();
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
            } finally {
                releaseObservationLock();
            }
        }, "orbit-data-recorder-shutdown"));
    }

    private static void acquireObservationLock() throws Exception {
        Path lockPath = instanceRoot.resolve(".orbit/runtime-data/observation.lock");
        Files.createDirectories(lockPath.getParent());
        observationLockChannel = FileChannel.open(
            lockPath,
            StandardOpenOption.CREATE,
            StandardOpenOption.WRITE
        );
        observationLock = observationLockChannel.tryLock();
        if (observationLock == null) {
            observationLockChannel.close();
            throw new IllegalStateException("another observed runtime is already active for this instance");
        }
    }

    private static void releaseObservationLock() {
        try {
            if (observationActiveMarker != null) Files.deleteIfExists(observationActiveMarker);
        } catch (Throwable ignored) {
        }
        try {
            if (observationLock != null) observationLock.release();
        } catch (Throwable ignored) {
        }
        try {
            if (observationLockChannel != null) observationLockChannel.close();
        } catch (Throwable ignored) {
        }
    }

    private static void publishObservationActivity() throws Exception {
        Path runtimeData = instanceRoot.resolve(".orbit/runtime-data");
        observationActiveMarker = runtimeData.resolve("observation.active");
        writeMarker(runtimeData.resolve("observation.epoch"), sessionId);
        writeMarker(observationActiveMarker, sessionId);
    }

    private static void writeMarker(Path path, String value) throws Exception {
        Path temporary = path.resolveSibling(path.getFileName() + ".tmp-" + sessionId);
        Files.write(
            temporary,
            value.getBytes(StandardCharsets.UTF_8),
            StandardOpenOption.CREATE_NEW,
            StandardOpenOption.WRITE
        );
        try {
            Files.move(temporary, path, StandardCopyOption.ATOMIC_MOVE, StandardCopyOption.REPLACE_EXISTING);
        } catch (java.nio.file.AtomicMoveNotSupportedException ignored) {
            Files.move(temporary, path, StandardCopyOption.REPLACE_EXISTING);
        }
    }

    private static Path allocateSessionFile(Path requested) {
        if (!Files.exists(requested)) {
            return requested;
        }
        String name = requested.getFileName().toString();
        String stem = name.endsWith(".events")
            ? name.substring(0, name.length() - ".events".length())
            : name;
        return requested.resolveSibling(stem + "-" + sessionId + ".events");
    }

    /** Resolve one class definition to the selected top-level package artifact. */
    public static String ownerFor(ProtectionDomain domain) {
        try {
            if (domain == null || domain.getCodeSource() == null) {
                return null;
            }
            if (quiltModuleIdentity) {
                String module = quiltModuleId(domain);
                if (module != null) {
                    String owner = MODULE_OWNERS.get(module);
                    if (owner != null) {
                        return owner;
                    }
                }
            }
            URL location = domain.getCodeSource().getLocation();
            final String identity = location.toExternalForm();
            String owner = SOURCE_CACHE.computeIfAbsent(identity, ignored -> sourceOwner(location));
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

    /** Fast path for repeated writes below an already-owned directory tree. */
    static boolean owns(Path rawPath, String owner) {
        Path path = normalize(rawPath);
        if (!validMutation(path, owner)) {
            return false;
        }
        State exact = STATES.get(normalizedString(path));
        if (exact != null) {
            if (exact.deleted()) return false;
            if (exact.owner != null) return owner.equals(exact.owner);
        }
        OwnedTree inherited = nearestTree(path);
        return inherited != null && inherited.owner.equals(owner);
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
            state.action = "delete";
            state.owner = owner;
            state.revision = nextMutation();
            STATES.put(key, state);
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
            state.action = "create";
            state.owner = owner;
            state.revision = nextMutation();
            STATES.put(key, state);
            if (tree) {
                OWNED_TREES.put(key, new OwnedTree(path, owner));
                compactSameOwnerDescendants(path, key, owner);
            }
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
            if (exact != null && !exact.deleted() && owner.equals(exact.owner)) {
                return;
            }
            OwnedTree inherited = nearestTree(path);
            if (inherited != null && inherited.owner.equals(owner)) {
                return;
            }
            State state = STATES.computeIfAbsent(key, ignored -> new State(path, tree));
            if (!state.deleted()) {
                state.action = "write";
                state.owner = owner;
                state.revision = nextMutation();
            }
        }
    }

    private static boolean validMutation(Path path, String owner) {
        return path != null
            && owner != null
            && !owner.trim().isEmpty()
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
                && (child.owner == null || owner.equals(child.owner))) {
                remove.add(entry.getKey());
            }
        }
        remove.forEach(STATES::remove);
    }

    private static void loadContext(Path context) throws Exception {
        List<String> lines = Files.readAllLines(context, StandardCharsets.UTF_8);
        if (lines.isEmpty() || !lines.get(0).equals("3\tcontext\tend")) {
            throw new IllegalArgumentException("invalid runtime ownership context header");
        }
        boolean javaCapability = false;
        for (int index = 1; index < lines.size(); index++) {
            String[] fields = lines.get(index).split("\t", -1);
            if (fields.length == 4 && fields[0].equals("capability") && fields[3].equals("end")) {
                if (fields[1].equals("java")) {
                    requireJavaRange(fields[2]);
                    javaCapability = true;
                } else if (fields[1].equals("source") && fields[2].equals("file")) {
                    fileCodeSources = true;
                } else if (fields[1].equals("source") && fields[2].equals("union")) {
                    unionCodeSources = true;
                } else if (fields[1].equals("module") && fields[2].equals("quilt")) {
                    quiltModuleIdentity = true;
                } else {
                    throw new IllegalArgumentException("unknown runtime ownership capability");
                }
            } else if (fields.length == 3 && fields[0].equals("system-library") && fields[2].equals("end")) {
                if (!fields[1].equals("fabric.systemLibraries") && !fields[1].equals("loader.systemLibraries")) {
                    throw new IllegalArgumentException("invalid Loader system-library property");
                }
            } else if (fields.length == 4 && fields[0].equals("source") && fields[3].equals("end")) {
                requireDigest(fields[1]);
                requireDigest(fields[2]);
                String previous = SOURCE_OWNERS.putIfAbsent(fields[1], fields[2]);
                if (previous != null && !previous.equals(fields[2])) {
                    SOURCE_OWNERS.remove(fields[1]);
                }
            } else if (fields.length == 4 && fields[0].equals("module") && fields[3].equals("end")) {
                String module = new String(Base64.getUrlDecoder().decode(fields[1]), StandardCharsets.UTF_8);
                requireDigest(fields[2]);
                String previous = MODULE_OWNERS.putIfAbsent(module, fields[2]);
                if (previous != null && !previous.equals(fields[2])) {
                    MODULE_OWNERS.remove(module);
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
        if (!javaCapability || (!fileCodeSources && !unionCodeSources && !quiltModuleIdentity)) {
            throw new IllegalArgumentException("runtime ownership context has no verified runtime strategy");
        }
    }

    private static void requireJavaRange(String value) {
        int separator = value.indexOf('-');
        if (separator <= 0 || separator == value.length() - 1) {
            throw new IllegalArgumentException("invalid verified Java range");
        }
        int minimum = Integer.parseInt(value.substring(0, separator));
        int maximum = Integer.parseInt(value.substring(separator + 1));
        int current = runtimeFeature();
        if (current < minimum || current > maximum) {
            throw new IllegalArgumentException(
                "Java " + current + " is outside the verified Runtime Agent range " + value
            );
        }
    }

    private static int runtimeFeature() {
        String value = System.getProperty("java.specification.version", "8");
        if (value.startsWith("1.")) value = value.substring(2);
        int end = 0;
        while (end < value.length() && Character.isDigit(value.charAt(end))) end++;
        try {
            return Integer.parseInt(value.substring(0, end));
        } catch (RuntimeException ignored) {
            return 8;
        }
    }

    private static void requireDigest(String value) {
        if (value.length() != 64 || !value.chars().allMatch(character -> Character.digit(character, 16) >= 0)) {
            throw new IllegalArgumentException("invalid SHA-256 in runtime ownership context");
        }
    }

    private static Path decodePath(String value) {
        Path path = java.nio.file.Paths.get(new String(Base64.getUrlDecoder().decode(value), StandardCharsets.UTF_8));
        Path normalized = normalize(path);
        if (normalized == null) {
            throw new IllegalArgumentException("runtime ownership context path is not physical");
        }
        return normalized;
    }

    private static String sourceOwner(URL location) {
        try {
            Path source = codeSourcePath(location);
            if (source == null || !Files.isRegularFile(source)) {
                return "";
            }
            String digest = sha256Unchecked(source);
            return digest.isEmpty() ? "" : valueOrEmpty(SOURCE_OWNERS.get(digest));
        } catch (Throwable ignored) {
            return "";
        }
    }

    private static Path codeSourcePath(URL location) throws Exception {
        String external = location.toExternalForm();
        boolean jarWrapped = false;
        while (external.startsWith("jar:")) {
            jarWrapped = true;
            external = external.substring(4);
        }
        if (jarWrapped) {
            int nested = external.indexOf("!/");
            if (nested >= 0) {
                external = external.substring(0, nested);
            }
        }
        URI uri = URI.create(external);
        if ("file".equalsIgnoreCase(uri.getScheme()) && fileCodeSources) {
            return java.nio.file.Paths.get(uri).toAbsolutePath().normalize();
        }
        if ("union".equalsIgnoreCase(uri.getScheme()) && unionCodeSources) {
            return ModuleAccess.unionPrimaryPath(uri);
        }
        return null;
    }

    private static String quiltModuleId(ProtectionDomain domain) {
        try {
            Object source = domain.getCodeSource();
            ClassLoader loader = source.getClass().getClassLoader();
            Class<?> quiltSource = Class.forName(
                "org.quiltmc.loader.impl.launch.common.QuiltCodeSource",
                false,
                loader
            );
            if (!quiltSource.isInstance(source)) {
                return null;
            }
            Object optional = quiltSource.getMethod("getQuiltMod").invoke(source);
            Class<?> optionalClass = Class.forName("java.util.Optional");
            if (!((Boolean) optionalClass.getMethod("isPresent").invoke(optional)).booleanValue()) {
                return null;
            }
            Object container = optionalClass.getMethod("get").invoke(optional);
            Class<?> containerApi = Class.forName("org.quiltmc.loader.api.ModContainer", false, loader);
            Object metadata = containerApi.getMethod("metadata").invoke(container);
            Class<?> metadataApi = Class.forName("org.quiltmc.loader.api.ModMetadata", false, loader);
            return (String) metadataApi.getMethod("id").invoke(metadata);
        } catch (Throwable ignored) {
            return null;
        }
    }

    private static String valueOrEmpty(String value) {
        return value == null ? "" : value;
    }

    private static String sha256Unchecked(Path path) {
        try {
            MessageDigest digest = MessageDigest.getInstance("SHA-256");
            try (java.io.InputStream input = Files.newInputStream(path)) {
                byte[] buffer = new byte[128 * 1024];
                int read;
                while ((read = input.read(buffer)) >= 0) {
                    digest.update(buffer, 0, read);
                }
            }
            byte[] bytes = digest.digest();
            StringBuilder output = new StringBuilder(bytes.length * 2);
            for (byte value : bytes) {
                output.append(Character.forDigit((value >>> 4) & 0x0f, 16));
                output.append(Character.forDigit(value & 0x0f, 16));
            }
            return output.toString();
        } catch (Throwable ignored) {
            return "";
        }
    }

    private static Path normalize(String path) {
        return path == null ? null : normalize(java.nio.file.Paths.get(path));
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
        return WINDOWS ? value.toLowerCase() : value;
    }

    private static long nextMutation() {
        return MUTATION_VERSION.incrementAndGet();
    }

    private static void markDirty() {
        nextMutation();
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
            List<State> snapshot = new ArrayList<State>(STATES.values());
            Collections.sort(snapshot, Comparator.comparing(state -> state.path.toString()));
            try (BufferedWriter writer = Files.newBufferedWriter(temporary, StandardCharsets.UTF_8)) {
                writer.write("3\tsnapshot\t");
                writer.write(sessionId);
                writer.write('\t');
                writer.write(Long.toString(sessionStartedAtMillis));
                writer.write('\t');
                writer.write(Long.toString(MUTATION_VERSION.get()));
                writer.write("\tend\n");
                for (State state : snapshot) {
                    if (state.action != null && state.owner != null) {
                        writeRecord(writer, state.action, state, state.owner);
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
        writer.write("3\t");
        writer.write(action);
        writer.write('\t');
        writer.write(state.tree ? "tree" : "file");
        writer.write('\t');
        writer.write(owner);
        writer.write('\t');
        writer.write(Long.toString(state.revision));
        writer.write('\t');
        writer.write(Base64.getUrlEncoder().withoutPadding().encodeToString(
            state.path.toString().getBytes(StandardCharsets.UTF_8)
        ));
        writer.write("\tend\n");
    }

    private static final class State {
        private final Path path;
        private final boolean tree;
        private volatile String action;
        private volatile String owner;
        private volatile long revision;

        private State(Path path, boolean tree) {
            this.path = path;
            this.tree = tree;
        }

        private boolean deleted() {
            return "delete".equals(action);
        }
    }

    private static final class OwnedTree {
        private final Path path;
        private final String owner;

        private OwnedTree(Path path, String owner) {
            this.path = path;
            this.owner = owner;
        }
    }
}
