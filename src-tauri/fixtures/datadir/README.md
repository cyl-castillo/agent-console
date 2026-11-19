# Data directories of earlier versions

Each folder is a data directory (`<data_local_dir>/agent-console/`) as an
earlier release wrote it. `services::upgrade_tests` copies one into a
temporary `XDG_DATA_HOME`, opens every store with the current build and
checks that nothing is lost and nothing is silently reset.

The files are **hand-written to the shapes of that release's structs**
(`git show v0.90.0:src-tauri/src/services/<store>.rs`), not copied from a
real install: a real directory holds terminal scrollback and project paths.
They carry no `schemaVersion` — stores were not versioned then — and use the
legacy keys that release still wrote (`claudeSessionId`).

When a store's schema changes, add the folder of the last release before the
change and extend the test: an upgrade is a promise about these files.
