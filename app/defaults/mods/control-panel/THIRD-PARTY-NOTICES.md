# Third-party notices

The working and blocked frame sequences in `mods/control-panel/frames.json` and its generated JavaScript twin come from **cell-motion 0.1.0**, commit `e528cb6eb7b6e70d633480cf6d03be58be3abb47`, by the upstream cell-motion authors.

Upstream declares **MIT** in both `README.md` and `package.json` at that commit. The inspected source has no standalone LICENSE file or copyright attribution to reproduce. This package retains that declaration and source/version provenance with the generated frames; it does not vendor the upstream implementation.

The inspected source is cell-motion 0.1.0, commit e528cb6. It was not edited, built, installed into or committed to for this package. The bake script requires its checkout path as an argument, imports the already-built `dist/terminal/index.js` and writes only into this package.

Comet and Breathe use the kit's terminal generators. Check's final `✓` is a static terminal adaptation of the kit's documented final-state design; it is not claimed to come from a nonexistent terminal Check generator. No upstream animation clock, React runtime or ANSI player ships in this mod.
