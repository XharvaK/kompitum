# Security and privacy

## Mods have your access

A mod is trusted JavaScript running inside Kompitum. It can run programs with your environment, fetch URLs and read files.
The built-in file API limits writes to the app home, but a mod can run a program to write elsewhere. That is not a security boundary.
Mods can also run programs on connected machines.

There is no permission prompt for each program or network request. Read a mod before installing it, and treat it like a shell script you would run yourself.
Layouts can load mods. Check those references too.

## What stays on disk

The default app home is `~/.config/kompitum-app`, including on macOS and Windows (under your user home).
`XDG_CONFIG_HOME` changes the config root. `KOMPITUM_APP_HOME` selects an app home directly and takes precedence.

This folder holds your settings, keys, layouts, mods, generated agent guide and API types.
It also holds mod data, shared state, window positions, this screen's focus and zoom, update notes and cached media, and telemetry while enabled.
Mods may store sensitive data here. Kompitum does not encrypt this folder.
The [installed guide's file map](../defaults/AGENTS.md#the-files) explains the files.

Spaces, tabs and splits are stored in each machine's core document, not in the app home.
Terminals and their output belong to [kompitum-core](https://github.com/XharvaK/kompitum).
Removing the app home does not stop the core or erase its data.

The app exposes a local control socket in its home so agents can inspect and drive the window.
Treat access to that home as access to the window and its commands.
Remote connections use SSH through the core.

## Updates and signing

Installed releases check the Kompitum release service, download an update, and check its SHA-256 checksum against the release manifest before unpacking it.
The app switches to the downloaded version on quit. Source builds do not update themselves.
Core updates are handled separately by the core's updater.

A checksum from the same service detects a damaged download; it is not an independent signature on the manifest.
The app does not verify a separate cryptographic signature on update manifests.

The release pipeline supports Apple Developer ID signing on macOS and Azure artifact signing on Windows when their credentials are configured.
Without them, packages can be unsigned or ad hoc signed.
On macOS the installer and updater verify certificate-signed bundles and preserve their signatures.
Apple notarization is tested separately; it is not required for publishing a release.
Do not assume every download is notarized or signed.

## Telemetry

Kompitum sends no usage data. It keeps a few counts on this computer (which features ran, updates, unclean exits) in `telemetry/` under its state folder, for you and for diagnostics; `"telemetry": false` in settings.json turns them off and deletes them.

The counts cover app and core versions, OS and architecture, bucketed usage time and counts, known agent names, built-in layout names, mod counts, app command names, and fixed event counters.
Custom layout names become `custom`; custom mod names are not recorded.
Terminal text, prompts, file contents, folder paths and free-text names are not included.

Turn them off with this field in your existing `settings.json`:

```json
{
  "telemetry": false
}
```

Turning it off deletes the local install ID and the counts. Nothing leaves this computer.
For one run, set `DO_NOT_TRACK=1` or `KOMPITUM_TELEMETRY=0`. CI environments disable collection too.
`KOMPITUM_TELEMETRY_URL` points a test build at a test server; ordinary builds never send.

Turning the counts off does not stop update checks or mod network requests.
Send feedback opens a filled-in GitHub issue in your browser, for you to read and submit; the app sends nothing itself.

[Back to Kompitum](../README.md)
