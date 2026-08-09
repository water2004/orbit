package dev.orbit.agent;

import java.io.File;
import java.io.FileDescriptor;
import java.io.FileNotFoundException;
import java.io.FileOutputStream;

public final class ObservedFileOutputStream extends FileOutputStream {
    public ObservedFileOutputStream(String name) throws FileNotFoundException {
        super(Recorder.beforePath(name));
        Recorder.write(java.nio.file.Path.of(name), Recorder.takeBefore());
    }
    public ObservedFileOutputStream(String name, boolean append) throws FileNotFoundException {
        super(Recorder.beforePath(name), append);
        Recorder.write(java.nio.file.Path.of(name), Recorder.takeBefore());
    }
    public ObservedFileOutputStream(File file) throws FileNotFoundException {
        super(Recorder.beforeFile(file));
        Recorder.write(file.toPath(), Recorder.takeBefore());
    }
    public ObservedFileOutputStream(File file, boolean append) throws FileNotFoundException {
        super(Recorder.beforeFile(file), append);
        Recorder.write(file.toPath(), Recorder.takeBefore());
    }
    public ObservedFileOutputStream(FileDescriptor descriptor) {
        super(descriptor);
    }
}
