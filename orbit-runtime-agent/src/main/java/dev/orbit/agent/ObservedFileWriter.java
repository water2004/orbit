package dev.orbit.agent;

import java.io.File;
import java.io.FileDescriptor;
import java.io.FileWriter;
import java.io.IOException;

public final class ObservedFileWriter extends FileWriter {
    public ObservedFileWriter(String name, String owner) throws IOException {
        super(Recorder.beforePath(name));
        Recorder.write(java.nio.file.Paths.get(name), Recorder.takeBefore(), Recorder.resolveOwner(owner));
    }
    public ObservedFileWriter(String name, boolean append, String owner) throws IOException {
        super(Recorder.beforePath(name), append);
        Recorder.write(java.nio.file.Paths.get(name), Recorder.takeBefore(), Recorder.resolveOwner(owner));
    }
    public ObservedFileWriter(File file, String owner) throws IOException {
        super(Recorder.beforeFile(file));
        Recorder.write(file.toPath(), Recorder.takeBefore(), Recorder.resolveOwner(owner));
    }
    public ObservedFileWriter(File file, boolean append, String owner) throws IOException {
        super(Recorder.beforeFile(file), append);
        Recorder.write(file.toPath(), Recorder.takeBefore(), Recorder.resolveOwner(owner));
    }
}
