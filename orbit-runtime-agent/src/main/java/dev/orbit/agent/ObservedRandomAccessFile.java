package dev.orbit.agent;

import java.io.File;
import java.io.FileNotFoundException;
import java.io.RandomAccessFile;

public final class ObservedRandomAccessFile extends RandomAccessFile {
    public ObservedRandomAccessFile(String name, String mode) throws FileNotFoundException {
        super(Recorder.beforePath(name), mode);
        boolean existed = Recorder.takeBefore();
        if (mode.equals("r")) Recorder.read(java.nio.file.Path.of(name));
        else Recorder.write(java.nio.file.Path.of(name), existed);
    }
    public ObservedRandomAccessFile(File file, String mode) throws FileNotFoundException {
        super(Recorder.beforeFile(file), mode);
        boolean existed = Recorder.takeBefore();
        if (mode.equals("r")) Recorder.read(file.toPath());
        else Recorder.write(file.toPath(), existed);
    }
}
