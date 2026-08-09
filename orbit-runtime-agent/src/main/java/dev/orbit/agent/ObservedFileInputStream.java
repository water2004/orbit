package dev.orbit.agent;

import java.io.File;
import java.io.FileDescriptor;
import java.io.FileInputStream;
import java.io.FileNotFoundException;

public final class ObservedFileInputStream extends FileInputStream {
    public ObservedFileInputStream(String name) throws FileNotFoundException {
        super(name);
        Recorder.read(java.nio.file.Path.of(name));
    }
    public ObservedFileInputStream(File file) throws FileNotFoundException {
        super(file);
        Recorder.read(file.toPath());
    }
    public ObservedFileInputStream(FileDescriptor descriptor) {
        super(descriptor);
    }
}
