package dev.orbit.agent;

import java.nio.file.Path;

/** Call-site hook for native stores whose internal writes bypass Java file APIs. */
public final class ObservedNativeStores {
    private ObservedNativeStores() {}

    public static String rocksDbPath(String value, String owner) {
        try {
            Path path = java.nio.file.Paths.get(value);
            boolean existed = java.nio.file.Files.exists(path);
            Recorder.tree(path, existed, true, owner);
        } catch (Throwable ignored) {
            // The original native API remains authoritative for invalid paths.
        }
        return value;
    }
}
