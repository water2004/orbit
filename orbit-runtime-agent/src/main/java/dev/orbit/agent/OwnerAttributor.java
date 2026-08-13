package dev.orbit.agent;

/** Resolves the logical caller when a package delegates I/O to a dependency. */
interface OwnerAttributor {
    String resolve(String directOwner);

    static OwnerAttributor create() {
        return new SecurityManagerOwnerAttributor();
    }
}
