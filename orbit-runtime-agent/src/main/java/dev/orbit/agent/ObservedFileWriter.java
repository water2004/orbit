package dev.orbit.agent;

import java.io.File;
import java.io.FileDescriptor;
import java.io.FileWriter;
import java.io.IOException;
import java.nio.charset.Charset;

public final class ObservedFileWriter extends FileWriter {
    public ObservedFileWriter(String name) throws IOException {
        super(Recorder.beforePath(name));
        Recorder.write(java.nio.file.Path.of(name), Recorder.takeBefore());
    }
    public ObservedFileWriter(String name, boolean append) throws IOException {
        super(Recorder.beforePath(name), append);
        Recorder.write(java.nio.file.Path.of(name), Recorder.takeBefore());
    }
    public ObservedFileWriter(File file) throws IOException {
        super(Recorder.beforeFile(file));
        Recorder.write(file.toPath(), Recorder.takeBefore());
    }
    public ObservedFileWriter(File file, boolean append) throws IOException {
        super(Recorder.beforeFile(file), append);
        Recorder.write(file.toPath(), Recorder.takeBefore());
    }
    public ObservedFileWriter(String name, Charset charset) throws IOException {
        super(Recorder.beforePath(name), charset);
        Recorder.write(java.nio.file.Path.of(name), Recorder.takeBefore());
    }
    public ObservedFileWriter(String name, Charset charset, boolean append) throws IOException {
        super(Recorder.beforePath(name), charset, append);
        Recorder.write(java.nio.file.Path.of(name), Recorder.takeBefore());
    }
    public ObservedFileWriter(File file, Charset charset) throws IOException {
        super(Recorder.beforeFile(file), charset);
        Recorder.write(file.toPath(), Recorder.takeBefore());
    }
    public ObservedFileWriter(File file, Charset charset, boolean append) throws IOException {
        super(Recorder.beforeFile(file), charset, append);
        Recorder.write(file.toPath(), Recorder.takeBefore());
    }
}
