// SPDX-License-Identifier: MPL-2.0

mod cgroup;
mod desktop;
mod platform;
mod steam;
#[cfg(test)]
mod tests;

use std::collections::{HashMap, HashSet};

use rustix::{
    io::Errno,
    process::{Signal, pidfd_send_signal},
};

use self::{
    cgroup::{CgroupInfo, desktop_id_from_user_unit, friendly_unit_name},
    desktop::{DesktopIndex, INDEX, friendly_identity, is_infrastructure},
    platform::{collect_snapshot, open_verified_pidfd, stop_user_units},
};

pub const WORKLOAD_LIMIT: usize = 7;

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ProcessIdentity {
    pub pid: u32,
    start_ticks: u64,
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum WorkloadKey {
    Container(String),
    Desktop(String),
    SteamGame(u32),
    UserUnit(String),
    Process(ProcessIdentity),
    ProcessTree(ProcessIdentity),
}

#[derive(Clone, Debug)]
pub struct WorkloadInfo {
    pub key: WorkloadKey,
    pub name: String,
    pub memory: u64,
    pub approximate_memory: bool,
    pub members: Vec<ProcessIdentity>,
}

#[derive(Clone, Debug)]
struct RawProcess {
    identity: ProcessIdentity,
    parent: Option<u32>,
    name: String,
    memory: u64,
    approximate_memory: bool,
    cmd: Option<String>,
    exe: Option<String>,
    container: Option<String>,
    cgroup: CgroupInfo,
    steam_game: Option<u32>,
    tmux_server: bool,
}

type ProcessSnapshot = HashMap<u32, RawProcess>;
type Classifications = HashMap<u32, WorkloadKey>;

pub fn scan_workloads() -> Result<Vec<WorkloadInfo>, String> {
    let snapshot = collect_snapshot(true)?;
    Ok(workloads_from_snapshot(&snapshot, &INDEX))
}

pub fn kill_workload(workload: &WorkloadInfo) -> Result<(), String> {
    let snapshot = collect_snapshot(false)?;
    let classifications = classify_all(&snapshot, &INDEX);
    let (mut identities, units) = targets_for(&workload.key, &snapshot, &classifications);
    identities.extend(&workload.members);
    identities.sort_unstable();
    identities.dedup();

    // Open every pidfd before changing systemd state. The fd pins the process,
    // while its start time proves that a stale PID was not reused.
    let mut targets = Vec::new();
    let mut errors = Vec::new();
    for identity in identities {
        match open_verified_pidfd(identity) {
            Ok(Some(fd)) => targets.push(fd),
            Ok(None) => {}
            Err(error) => errors.push(format!("PID {}: {error}", identity.pid)),
        }
    }

    if !units.is_empty()
        && let Err(error) = stop_user_units(&units)
    {
        errors.push(format!("could not stop user service: {error}"));
    }
    for target in targets {
        if let Err(error) = pidfd_send_signal(target, Signal::KILL)
            && error != Errno::SRCH
        {
            errors.push(format!("could not send SIGKILL: {error}"));
        }
    }

    errors
        .is_empty()
        .then_some(())
        .ok_or_else(|| errors.join("; "))
}

fn workloads_from_snapshot(
    snapshot: &ProcessSnapshot,
    desktop_index: &DesktopIndex,
) -> Vec<WorkloadInfo> {
    let classifications = classify_all(snapshot, desktop_index);
    let mut grouped: HashMap<WorkloadKey, WorkloadInfo> = HashMap::new();

    for process in snapshot.values() {
        let key = &classifications[&process.identity.pid];
        let workload = grouped.entry(key.clone()).or_insert_with(|| WorkloadInfo {
            key: key.clone(),
            name: display_name(key, process, desktop_index),
            memory: 0,
            approximate_memory: false,
            members: Vec::new(),
        });
        workload.memory = workload.memory.saturating_add(process.memory);
        workload.approximate_memory |= process.approximate_memory;
        workload.members.push(process.identity);
    }

    let mut workloads = grouped
        .into_values()
        .filter(|workload| !is_infrastructure(workload))
        .collect::<Vec<_>>();
    for workload in &mut workloads {
        workload.members.sort_unstable();
    }
    workloads.sort_by(|left, right| {
        right
            .memory
            .cmp(&left.memory)
            .then_with(|| left.name.cmp(&right.name))
            .then_with(|| left.key.cmp(&right.key))
    });
    workloads.truncate(WORKLOAD_LIMIT);
    workloads
}

fn classify_all(snapshot: &ProcessSnapshot, desktop_index: &DesktopIndex) -> Classifications {
    let mut memo = HashMap::new();
    for pid in snapshot.keys().copied() {
        classify_pid(pid, snapshot, desktop_index, &mut memo);
    }
    memo
}

fn classify_pid(
    pid: u32,
    snapshot: &ProcessSnapshot,
    desktop_index: &DesktopIndex,
    memo: &mut Classifications,
) -> Option<WorkloadKey> {
    if let Some(classification) = memo.get(&pid) {
        return Some(classification.clone());
    }
    let process = snapshot.get(&pid)?;

    let inherited = process.parent.and_then(|parent| {
        classify_pid(parent, snapshot, desktop_index, memo)
            .filter(|key| !matches!(key, WorkloadKey::Process(_)))
    });
    let classification = identity_classification(process, desktop_index)
        .or(inherited)
        .or_else(|| cgroup_classification(process, desktop_index))
        .unwrap_or(WorkloadKey::Process(process.identity));

    memo.insert(pid, classification.clone());
    Some(classification)
}

fn identity_classification(
    process: &RawProcess,
    desktop_index: &DesktopIndex,
) -> Option<WorkloadKey> {
    process
        .steam_game
        .map(WorkloadKey::SteamGame)
        .or_else(|| {
            process
                .tmux_server
                .then_some(WorkloadKey::ProcessTree(process.identity))
        })
        .or_else(|| process.container.clone().map(WorkloadKey::Container))
        .or_else(|| {
            desktop_index
                .by_process_executable(process)
                .map(|app| WorkloadKey::Desktop(app.id.clone()))
        })
}

fn cgroup_classification(
    process: &RawProcess,
    desktop_index: &DesktopIndex,
) -> Option<WorkloadKey> {
    let app = process
        .cgroup
        .user_unit
        .as_deref()
        .and_then(desktop_id_from_user_unit)
        .and_then(|id| desktop_index.by_desktop_id(&id))
        .or_else(|| {
            process
                .cgroup
                .app_scope_id
                .as_deref()
                .and_then(|hint| desktop_index.by_scope_hint(hint))
        });
    app.map(|app| WorkloadKey::Desktop(app.id.clone()))
        .or_else(|| process.cgroup.user_unit.clone().map(WorkloadKey::UserUnit))
}

fn display_name(key: &WorkloadKey, process: &RawProcess, index: &DesktopIndex) -> String {
    match key {
        WorkloadKey::Container(id) | WorkloadKey::Desktop(id) => index
            .by_desktop_id(id)
            .map(|app| app.name.clone())
            .unwrap_or_else(|| friendly_identity(id)),
        WorkloadKey::SteamGame(id) => steam::game_name(*id),
        WorkloadKey::UserUnit(unit) => friendly_unit_name(unit),
        WorkloadKey::Process(_) => process.name.clone(),
        WorkloadKey::ProcessTree(_) => "tmux".into(),
    }
}

fn targets_for(
    key: &WorkloadKey,
    snapshot: &ProcessSnapshot,
    classifications: &Classifications,
) -> (Vec<ProcessIdentity>, Vec<String>) {
    let mut identities = Vec::new();
    let mut units = HashSet::new();
    for process in snapshot.values() {
        if classifications.get(&process.identity.pid) == Some(key) {
            identities.push(process.identity);
            units.extend(process.cgroup.user_unit.clone());
        }
    }
    units.retain(|unit| {
        snapshot
            .values()
            .filter(|process| process.cgroup.user_unit.as_ref() == Some(unit))
            .all(|process| classifications.get(&process.identity.pid) == Some(key))
    });
    (identities, units.into_iter().collect())
}
