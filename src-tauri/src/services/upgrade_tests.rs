//! An upgrade is a promise about files that already exist on someone's disk.
//! These tests open the data directory of an earlier release (see
//! `fixtures/datadir/README.md`) with the current build and check that every
//! store reads all of it, and that a save by the current build loses nothing.

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::services::notes_service::NotesService;
use crate::services::roundtable_service::RoomsStore;
use crate::services::scheduler_service::SchedulerService;
use crate::services::sessions_service::SessionsService;

const PROJECT: &str = "/proj/demo";

fn fixture(release: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures/datadir")
        .join(release)
        .join("agent-console")
}

/// Copy a release's data directory into a fresh XDG_DATA_HOME.
fn install(release: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let base = std::env::temp_dir().join(format!(
        "ac-upgrade-{release}-{}-{nanos}",
        std::process::id()
    ));
    let dest = base.join("agent-console");
    fs::create_dir_all(&dest).unwrap();
    for entry in fs::read_dir(fixture(release)).unwrap() {
        let entry = entry.unwrap();
        fs::copy(entry.path(), dest.join(entry.file_name())).unwrap();
    }
    std::env::set_var("XDG_DATA_HOME", &base);
    base
}

fn read_json(path: &Path) -> Value {
    serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap()
}

/// One fn on purpose: the stores read XDG_DATA_HOME, which is process-wide.
#[test]
fn a_data_directory_from_v0_90_opens_whole_and_survives_a_save() {
    let _env = crate::test_support::lock_env();
    let base = install("v0.90");
    let dir = base.join("agent-console");

    // ---- sessions -------------------------------------------------------
    let sessions = SessionsService::new();
    let (list, quarantined) = sessions.list_recovering(PROJECT).unwrap();
    assert!(quarantined.is_none(), "an old file is not a corrupt file");
    assert_eq!(list.len(), 2);
    let one = list.iter().find(|s| s.id == "s-one").unwrap();
    assert_eq!(
        one.agent_session_id.as_deref(),
        Some("0c1d2e3f-legacy-key"),
        "the legacy claudeSessionId key still binds the conversation"
    );
    assert!(one.scrollback.contains("14 passed 🚀"));
    let two = list.iter().find(|s| s.id == "s-two").unwrap();
    assert_eq!(two.archived, Some(true));
    assert_eq!(
        two.worktree.as_ref().map(|w| w.branch.as_str()),
        Some("session/s-two")
    );
    assert_eq!(sessions.list("/proj/other").unwrap().len(), 1);

    // A save by this build keeps everything, for this project and the others.
    sessions.save(PROJECT, list).unwrap();
    let after = sessions.list(PROJECT).unwrap();
    assert_eq!(after.len(), 2);
    assert_eq!(
        after
            .iter()
            .find(|s| s.id == "s-one")
            .unwrap()
            .agent_session_id
            .as_deref(),
        Some("0c1d2e3f-legacy-key")
    );
    assert_eq!(sessions.list("/proj/other").unwrap().len(), 1);
    let raw = read_json(&dir.join("sessions.json"));
    assert_eq!(raw["schemaVersion"], 1, "the saved file is now versioned");
    assert!(raw["by_project"].is_object(), "and keeps its layout");
    // The pre-upgrade file is what the backup holds: one step back is possible.
    assert_eq!(
        read_json(&dir.join("sessions.json.bak")),
        read_json(&fixture("v0.90").join("sessions.json"))
    );

    // ---- notes ----------------------------------------------------------
    let notes = NotesService::new();
    let list = notes.list(PROJECT).unwrap();
    assert_eq!(list.len(), 2);
    assert_eq!(list[0].text, "revisar el token vencido");
    notes.save(PROJECT, list).unwrap();
    assert_eq!(notes.list(PROJECT).unwrap().len(), 2);
    assert_eq!(read_json(&dir.join("notes.json"))["schemaVersion"], 1);

    // ---- scheduler ------------------------------------------------------
    let scheduler = SchedulerService::new();
    let jobs = scheduler.list(PROJECT).unwrap();
    assert_eq!(jobs.len(), 2);
    let daily = jobs.iter().find(|j| j.id == "j-daily").unwrap();
    assert!(daily.enabled);
    assert_eq!(daily.last_run_ms, Some(1_790_000_400_000));
    let skill = jobs.iter().find(|j| j.id == "j-skill").unwrap();
    assert!(!skill.enabled, "a disabled job stays disabled");

    // ---- rooms ----------------------------------------------------------
    let rooms = RoomsStore::new();
    let room = rooms.get(PROJECT, "room-1").unwrap().expect("room-1");
    assert_eq!(room.participants.len(), 2);
    assert_eq!(room.transcript.len(), 2);
    assert_eq!(room.transcript[1].text, "the fixture clock is frozen");
    assert_eq!(room.resume.get("p1").map(String::as_str), Some("sess-abc"));
    assert_eq!(room.base_branch.as_deref(), Some("main"));
    rooms.save_room(&room, Path::new(PROJECT)).unwrap();
    let again = rooms.get(PROJECT, "room-1").unwrap().expect("room-1");
    assert_eq!(again.transcript.len(), 2);
    assert_eq!(again.total_tokens, 1234);
    assert_eq!(read_json(&dir.join("rooms.json"))["schemaVersion"], 1);

    let _ = fs::remove_dir_all(&base);
}
