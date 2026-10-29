import { createHash } from "node:crypto";
import { resolve } from "node:path";

process.chdir(resolve(import.meta.dir, ".."));
const inventory = await Bun.file("licenses/inventory.json").json();
const inputs = [...new Set<string>([
  ...inventory.inputs,
  ...inventory.components.flatMap((component: { files: string[] }) => component.files),
])].sort();

const tool = process.env.CARGO_ABOUT ?? "cargo-about";
const version = Bun.spawnSync([tool, "--version"]);
if (version.exitCode !== 0 || version.stdout.toString().trim() !== "cargo-about 0.9.3") {
  throw new Error("Install cargo-about 0.9.3 (see licenses/README.md)");
}
const generated = Bun.spawnSync([
  tool, "generate", "--locked", "--fail", "scripts/notices.hbs",
], { stdout: "pipe", stderr: "inherit" });
if (generated.exitCode !== 0) throw new Error("cargo-about failed");

let text = "Herdr third-party notices\n=========================\n"
  + "Regenerate with just notices. See licenses/README.md.\n\n";
for (const path of inputs) {
  // Normalize checkout line endings so the same check works on Windows.
  const content = (await Bun.file(path).text()).replaceAll("\r\n", "\n");
  const hash = createHash("sha256").update(content).digest("hex");
  text += `Input-SHA256: ${hash} ${path}\n`;
}
text += "\n" + generated.stdout.toString().replaceAll("\r\n", "\n");
for (const component of inventory.components) {
  text += `\n----------------------------------------------------------------------\n${component.name}\n${component.source}\n\n`;
  for (const path of component.files) {
    text += `--- ${path} ---\n${(await Bun.file(path).text()).replaceAll("\r\n", "\n").trim()}\n\n`;
  }
}
await Bun.write("THIRD-PARTY-NOTICES", text);
console.log("Wrote THIRD-PARTY-NOTICES");
