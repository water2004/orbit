/** Package code delegating persistence to a declared library dependency. */
public final class AgentDelegateConsumer {
    private AgentDelegateConsumer() {}

    public static void main(String[] arguments) throws Exception {
        if (arguments.length == 1) {
            AgentDelegateLibrary.write(arguments[0]);
            return;
        }

        int iterations = Integer.parseInt(arguments[1]);
        int warmup = Math.min(iterations, 1_000);
        for (int index = 0; index < warmup; index++) {
            AgentDelegateLibrary.write(arguments[0]);
        }

        long started = System.nanoTime();
        for (int index = 0; index < iterations; index++) {
            AgentDelegateLibrary.write(arguments[0]);
        }
        long elapsed = System.nanoTime() - started;

        Runtime runtime = Runtime.getRuntime();
        long usedHeap = runtime.totalMemory() - runtime.freeMemory();
        System.out.println(
            "{\"iterations\":" + iterations
                + ",\"elapsed_nanos\":" + elapsed
                + ",\"used_heap_bytes\":" + usedHeap + "}"
        );
    }
}
