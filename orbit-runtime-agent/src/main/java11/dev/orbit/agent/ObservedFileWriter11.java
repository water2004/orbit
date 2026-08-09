package dev.orbit.agent;

import java.io.File;
import java.io.FileWriter;
import java.io.IOException;
import java.nio.charset.Charset;
import java.nio.file.Paths;

/** Charset-aware FileWriter replacement loaded only on Java 11 or newer. */
public final class ObservedFileWriter11 extends FileWriter {
    public ObservedFileWriter11(String name, String owner) throws IOException {
        super(Recorder.beforePath(name));
        Recorder.write(Paths.get(name), Recorder.takeBefore(), owner);
    }
    public ObservedFileWriter11(String name, boolean append, String owner) throws IOException {
        super(Recorder.beforePath(name), append);
        Recorder.write(Paths.get(name), Recorder.takeBefore(), owner);
    }
    public ObservedFileWriter11(File file, String owner) throws IOException {
        super(Recorder.beforeFile(file));
        Recorder.write(file.toPath(), Recorder.takeBefore(), owner);
    }
    public ObservedFileWriter11(File file, boolean append, String owner) throws IOException {
        super(Recorder.beforeFile(file), append);
        Recorder.write(file.toPath(), Recorder.takeBefore(), owner);
    }
    public ObservedFileWriter11(String name, Charset charset, String owner) throws IOException {
        super(Recorder.beforePath(name), charset);
        Recorder.write(Paths.get(name), Recorder.takeBefore(), owner);
    }
    public ObservedFileWriter11(String name, Charset charset, boolean append, String owner) throws IOException {
        super(Recorder.beforePath(name), charset, append);
        Recorder.write(Paths.get(name), Recorder.takeBefore(), owner);
    }
    public ObservedFileWriter11(File file, Charset charset, String owner) throws IOException {
        super(Recorder.beforeFile(file), charset);
        Recorder.write(file.toPath(), Recorder.takeBefore(), owner);
    }
    public ObservedFileWriter11(File file, Charset charset, boolean append, String owner) throws IOException {
        super(Recorder.beforeFile(file), charset, append);
        Recorder.write(file.toPath(), Recorder.takeBefore(), owner);
    }
}
