package dev.orbit.agent;

import java.io.File;
import java.io.FileDescriptor;
import java.io.FileNotFoundException;
import java.io.FileReader;
import java.nio.charset.Charset;

public final class ObservedFileReader extends FileReader {
    public ObservedFileReader(String name) throws FileNotFoundException {
        super(name);
        Recorder.read(java.nio.file.Path.of(name));
    }
    public ObservedFileReader(File file) throws FileNotFoundException {
        super(file);
        Recorder.read(file.toPath());
    }
    public ObservedFileReader(FileDescriptor descriptor) {
        super(descriptor);
    }
    public ObservedFileReader(String name, Charset charset) throws java.io.IOException {
        super(name, charset);
        Recorder.read(java.nio.file.Path.of(name));
    }
    public ObservedFileReader(File file, Charset charset) throws java.io.IOException {
        super(file, charset);
        Recorder.read(file.toPath());
    }
}
