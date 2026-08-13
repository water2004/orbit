package dev.orbit.agent;

import java.io.File;
import java.io.FileNotFoundException;
import java.io.RandomAccessFile;

public final class ObservedRandomAccessFile extends RandomAccessFile {
    public ObservedRandomAccessFile(String name, String mode, String owner) throws FileNotFoundException {
        super(Recorder.beforePath(name), mode);
        boolean existed = Recorder.takeBefore();
        if (!mode.equals("r")) Recorder.write(java.nio.file.Paths.get(name), existed, Recorder.resolveOwner(owner));
    }
    public ObservedRandomAccessFile(File file, String mode, String owner) throws FileNotFoundException {
        super(Recorder.beforeFile(file), mode);
        boolean existed = Recorder.takeBefore();
        if (!mode.equals("r")) Recorder.write(file.toPath(), existed, Recorder.resolveOwner(owner));
    }
}
