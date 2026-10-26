import { expect, it } from "vitest";
import { packageCommands, accountCommands } from "../src/services/commands";
import contract from "../../../orbit-machine-protocol/tests/fixtures/command-contract.json";

it("GUI command builders match the contract parsed by both Rust CLIs", () => {
  const pkg = packageCommands("sodium");
  const actual = [
    pkg.versions(),
    pkg.files(),
    pkg.activation(true),
    pkg.activation(false),
    pkg.reset(),
    pkg.purge(),
    pkg.environment("auto"),
    pkg.constraint(
      { kind: "comparison", operator: "=", version: "0.9.1" },
      "all",
    ),
    pkg.addRemote("modrinth", "AANobbMI"),
    pkg.removeRemote(1),
    accountCommands.default("player"),
    accountCommands.default(),
    accountCommands.instance("player"),
    accountCommands.instance(),
  ];
  const ids = new Set(actual.map((args) => JSON.stringify(args)));
  expect(ids.size).toBe(actual.length);
  for (const args of actual)
    expect(
      contract.some(
        (entry) => JSON.stringify(entry.args) === JSON.stringify(args),
      ),
    ).toBe(true);
});
