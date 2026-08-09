package dev.orbit.agent;

import java.lang.instrument.ClassFileTransformer;
import java.security.ProtectionDomain;
import java.util.Map;
import java.util.Set;

import net.bytebuddy.jar.asm.ClassReader;
import net.bytebuddy.jar.asm.ClassVisitor;
import net.bytebuddy.jar.asm.ClassWriter;
import net.bytebuddy.jar.asm.MethodVisitor;
import net.bytebuddy.jar.asm.Opcodes;
import net.bytebuddy.jar.asm.Type;

/** Rewrites file API calls in application classes; JDK classes remain untouched. */
public final class FileCallTransformer implements ClassFileTransformer {
    private static final String AGENT_PREFIX = "dev/orbit/agent/";
    private static final String OBSERVED_FILES = AGENT_PREFIX + "ObservedFiles";
    private static final String OBSERVED_CHANNELS = AGENT_PREFIX + "ObservedChannels";
    private static final String OBSERVED_NATIVE_STORES = AGENT_PREFIX + "ObservedNativeStores";

    private static final Map<String, String> OBSERVED_CONSTRUCTORS = Map.of(
        "java/io/FileOutputStream", AGENT_PREFIX + "ObservedFileOutputStream",
        "java/io/FileWriter", AGENT_PREFIX + "ObservedFileWriter",
        "java/io/RandomAccessFile", AGENT_PREFIX + "ObservedRandomAccessFile"
    );

    private static final Set<String> FILE_INSTANCE_METHODS = Set.of(
        "createNewFile()Z",
        "delete()Z",
        "mkdir()Z",
        "mkdirs()Z",
        "renameTo(Ljava/io/File;)Z"
    );

    public FileCallTransformer() {}

    @Override
    public byte[] transform(
        Module module,
        ClassLoader loader,
        String className,
        Class<?> classBeingRedefined,
        ProtectionDomain protectionDomain,
        byte[] classfileBuffer
    ) {
        if (className == null
            || className.startsWith(AGENT_PREFIX)
            || className.startsWith("java/")
            || className.startsWith("javax/")
            || className.startsWith("jdk/")
            || className.startsWith("sun/")
            || className.startsWith("net/bytebuddy/")) {
            return null;
        }
        String owner = Recorder.ownerFor(protectionDomain);
        if (owner == null) {
            return null;
        }
        try {
            var reader = new ClassReader(classfileBuffer);
            var writer = new ClassWriter(reader, ClassWriter.COMPUTE_MAXS);
            var visitor = new Rewriter(writer, owner);
            reader.accept(visitor, 0);
            return visitor.changed ? writer.toByteArray() : null;
        } catch (Throwable ignored) {
            // One untransformable class must not prevent Minecraft from starting.
            return null;
        }
    }

    private static final class Rewriter extends ClassVisitor {
        private boolean changed;
        private final String packageOwner;

        private Rewriter(ClassVisitor visitor, String packageOwner) {
            super(Opcodes.ASM9, visitor);
            this.packageOwner = packageOwner;
        }

        @Override
        public MethodVisitor visitMethod(
            int access,
            String name,
            String descriptor,
            String signature,
            String[] exceptions
        ) {
            MethodVisitor delegate = super.visitMethod(access, name, descriptor, signature, exceptions);
            return new MethodVisitor(Opcodes.ASM9, delegate) {
                @Override
                public void visitTypeInsn(int opcode, String type) {
                    String replacement = opcode == Opcodes.NEW ? OBSERVED_CONSTRUCTORS.get(type) : null;
                    if (replacement != null) {
                        changed = true;
                        super.visitTypeInsn(opcode, replacement);
                    } else {
                        super.visitTypeInsn(opcode, type);
                    }
                }

                @Override
                public void visitMethodInsn(
                    int opcode,
                    String owner,
                    String methodName,
                    String methodDescriptor,
                    boolean isInterface
                ) {
                    String constructor = OBSERVED_CONSTRUCTORS.get(owner);
                    if (constructor != null && opcode == Opcodes.INVOKESPECIAL && methodName.equals("<init>")) {
                        changed = true;
                        super.visitLdcInsn(packageOwner);
                        super.visitMethodInsn(
                            opcode,
                            constructor,
                            methodName,
                            appendOwner(methodDescriptor),
                            false
                        );
                        return;
                    }
                    if (owner.equals("java/nio/file/Files")
                        && opcode == Opcodes.INVOKESTATIC
                        && ObservedFiles.supports(methodName, methodDescriptor)) {
                        changed = true;
                        super.visitLdcInsn(packageOwner);
                        super.visitMethodInsn(
                            opcode,
                            OBSERVED_FILES,
                            methodName,
                            appendOwner(methodDescriptor),
                            false
                        );
                        return;
                    }
                    if ((owner.equals("java/nio/channels/FileChannel")
                            || owner.equals("java/nio/channels/AsynchronousFileChannel"))
                        && opcode == Opcodes.INVOKESTATIC
                        && methodName.equals("open")
                        && ObservedChannels.supports(owner, methodDescriptor)) {
                        changed = true;
                        super.visitLdcInsn(packageOwner);
                        super.visitMethodInsn(
                            opcode,
                            OBSERVED_CHANNELS,
                            owner.equals("java/nio/channels/FileChannel") ? "fileOpen" : "asyncOpen",
                            appendOwner(methodDescriptor),
                            false
                        );
                        return;
                    }
                    Type[] arguments = Type.getArgumentTypes(methodDescriptor);
                    if (owner.equals("org/rocksdb/RocksDB")
                        && opcode == Opcodes.INVOKESTATIC
                        && methodName.equals("open")
                        && arguments.length > 0
                        && arguments[arguments.length - 1].equals(Type.getType(String.class))) {
                        changed = true;
                        super.visitLdcInsn(packageOwner);
                        super.visitMethodInsn(
                            Opcodes.INVOKESTATIC,
                            OBSERVED_NATIVE_STORES,
                            "rocksDbPath",
                            "(Ljava/lang/String;Ljava/lang/String;)Ljava/lang/String;",
                            false
                        );
                    }
                    String fileKey = methodName + methodDescriptor;
                    if (owner.equals("java/io/File")
                        && opcode == Opcodes.INVOKEVIRTUAL
                        && FILE_INSTANCE_METHODS.contains(fileKey)) {
                        changed = true;
                        String staticDescriptor = "(Ljava/io/File;"
                            + methodDescriptor.substring(1);
                        super.visitLdcInsn(packageOwner);
                        super.visitMethodInsn(
                            Opcodes.INVOKESTATIC,
                            OBSERVED_FILES,
                            "file" + Character.toUpperCase(methodName.charAt(0)) + methodName.substring(1),
                            appendOwner(staticDescriptor),
                            false
                        );
                        return;
                    }
                    super.visitMethodInsn(opcode, owner, methodName, methodDescriptor, isInterface);
                }
            };
        }

        private static String appendOwner(String descriptor) {
            int end = descriptor.indexOf(')');
            return descriptor.substring(0, end)
                + "Ljava/lang/String;"
                + descriptor.substring(end);
        }
    }
}
