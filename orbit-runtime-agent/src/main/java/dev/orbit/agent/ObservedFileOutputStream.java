package dev.orbit.agent;

import java.io.File;
import java.io.FileDescriptor;
import java.io.FileNotFoundException;
import java.io.FileOutputStream;

public final class ObservedFileOutputStream extends FileOutputStream {
    public ObservedFileOutputStream(String name, String owner) throws FileNotFoundException {
        super(Recorder.beforePath(name));
        Recorder.write(java.nio.file.Paths.get(name), Recorder.takeBefore(), owner);
    }
    public ObservedFileOutputStream(String name, boolean append, String owner) throws FileNotFoundException {
        super(Recorder.beforePath(name), append);
        Recorder.write(java.nio.file.Paths.get(name), Recorder.takeBefore(), owner);
    }
    public ObservedFileOutputStream(File file, String owner) throws FileNotFoundException {
        super(Recorder.beforeFile(file));
        Recorder.write(file.toPath(), Recorder.takeBefore(), owner);
    }
    public ObservedFileOutputStream(File file, boolean append, String owner) throws FileNotFoundException {
        super(Recorder.beforeFile(file), append);
        Recorder.write(file.toPath(), Recorder.takeBefore(), owner);
    }
    public ObservedFileOutputStream(FileDescriptor descriptor, String owner) {
        super(descriptor);
    }
}
