package dev.orbit.agent;

/** Allocation-light class-context walk supported by the verified Java 8-25 range. */
@SuppressWarnings("removal")
final class SecurityManagerOwnerAttributor extends SecurityManager implements OwnerAttributor {
    @Override
    public String resolve(String directOwner) {
        String selected = directOwner;
        boolean foundDirectOwner = false;
        for (Class<?> type : getClassContext()) {
            String candidate = Recorder.ownerForClass(type);
            if (!foundDirectOwner) {
                if (directOwner.equals(candidate)) {
                    foundDirectOwner = true;
                }
                continue;
            }
            if (candidate == null) {
                break;
            }
            if (selected.equals(candidate)) {
                continue;
            }
            if (!Recorder.delegates(candidate, selected)) {
                break;
            }
            selected = candidate;
        }
        return selected;
    }
}
