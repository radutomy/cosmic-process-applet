// SPDX-License-Identifier: MPL-2.0

use std::{fs, os::fd::OwnedFd, path::Path};

use rustix::{
    io::Errno,
    process::{Pid, PidfdFlags, pidfd_open},
};
use sysinfo::{Process, ProcessRefreshKind, RefreshKind, System, Uid, UpdateKind};
use zbus::zvariant::OwnedObjectPath;

use super::{CgroupInfo, ProcessIdentity, ProcessSnapshot, RawProcess, steam::steam_game};

pub(super) fn collect_snapshot(with_memory: bool) -> Result<ProcessSnapshot, String> {
    let mut system = System::new();
    system.refresh_specifics(
        RefreshKind::nothing().with_processes(
            ProcessRefreshKind::nothing()
                .with_memory()
                .with_user(UpdateKind::OnlyIfNotSet)
                .with_cmd(UpdateKind::OnlyIfNotSet)
                .with_exe(UpdateKind::OnlyIfNotSet)
                .with_environ(UpdateKind::OnlyIfNotSet),
        ),
    );

    let current_uid = Uid::try_from(unsafe { libc::geteuid() } as usize)
        .map_err(|_| "could not determine the current user ID".to_string())?;
    let own_pid = std::process::id();
    let mut processes = ProcessSnapshot::new();

    for process in system.processes().values() {
        let pid = process.pid().as_u32();
        if process.thread_kind().is_some()
            || pid == own_pid
            || process.effective_user_id().or(process.user_id()) != Some(&current_uid)
        {
            continue;
        }

        let cgroup = fs::read_to_string(format!("/proc/{pid}/cgroup"))
            .map(|contents| CgroupInfo::parse(&contents))
            .unwrap_or_default();
        if cgroup.ignored {
            continue;
        }
        let Some(start_ticks) = process_start_ticks(pid) else {
            // Exiting processes are normal during a scan, but without a stable
            // identity they must never become killable targets.
            continue;
        };

        let (memory, approximate_memory) = if with_memory {
            proportional_memory(pid)
                .map(|memory| (memory, false))
                .unwrap_or_else(|| (process.memory(), true))
        } else {
            (0, false)
        };
        processes.insert(
            pid,
            RawProcess {
                identity: ProcessIdentity { pid, start_ticks },
                parent: process.parent().map(sysinfo::Pid::as_u32),
                name: best_process_name(process),
                memory,
                approximate_memory,
                cmd: process
                    .cmd()
                    .first()
                    .map(|value| value.to_string_lossy().into_owned()),
                exe: process
                    .exe()
                    .map(|value| value.to_string_lossy().into_owned()),
                container: container_identity(process),
                cgroup,
                steam_game: steam_game(process),
                tmux_server: process.name() == "tmux: server",
            },
        );
    }
    Ok(processes)
}

pub(super) fn open_verified_pidfd(identity: ProcessIdentity) -> Result<Option<OwnedFd>, String> {
    let raw_pid = i32::try_from(identity.pid).map_err(|_| "invalid PID".to_string())?;
    let pid = Pid::from_raw(raw_pid).ok_or_else(|| "invalid PID".to_string())?;
    let fd = match pidfd_open(pid, PidfdFlags::empty()) {
        Ok(fd) => fd,
        Err(Errno::SRCH | Errno::NOENT) => return Ok(None),
        Err(error) => return Err(error.to_string()),
    };
    if process_start_ticks(identity.pid) != Some(identity.start_ticks) {
        return Ok(None);
    }
    Ok(Some(fd))
}

pub(super) fn stop_user_units(units: &[String]) -> zbus::Result<()> {
    let connection = zbus::blocking::Connection::session()?;
    let proxy = zbus::blocking::Proxy::new(
        &connection,
        "org.freedesktop.systemd1",
        "/org/freedesktop/systemd1",
        "org.freedesktop.systemd1.Manager",
    )?;
    for unit in units {
        let _: OwnedObjectPath = proxy.call("StopUnit", &(unit, "replace"))?;
    }
    Ok(())
}

fn best_process_name(process: &Process) -> String {
    process
        .exe()
        .and_then(|path| path.file_name())
        .or_else(|| {
            process
                .cmd()
                .first()
                .and_then(|value| Path::new(value).file_name())
        })
        .unwrap_or_else(|| process.name())
        .to_string_lossy()
        .into_owned()
}

fn container_identity(process: &Process) -> Option<String> {
    const KEYS: [&str; 3] = ["FLATPAK_ID", "SNAP_NAME", "APPIMAGE"];
    process.environ().iter().find_map(|entry| {
        let entry = entry.to_string_lossy();
        let (key, value) = entry.split_once('=')?;
        KEYS.contains(&key)
            .then(|| value.strip_suffix(".desktop").unwrap_or(value).to_string())
    })
}

pub(super) fn process_start_ticks(pid: u32) -> Option<u64> {
    let stat = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    stat.rsplit_once(')')?
        .1
        .split_whitespace()
        .nth(19)?
        .parse()
        .ok()
}

fn proportional_memory(pid: u32) -> Option<u64> {
    let smaps = fs::read_to_string(format!("/proc/{pid}/smaps_rollup")).ok()?;
    proportional_memory_from_smaps_rollup(&smaps)
}

pub(super) fn proportional_memory_from_smaps_rollup(smaps: &str) -> Option<u64> {
    let kibibytes = smaps.lines().find_map(|line| {
        line.strip_prefix("Pss:")?
            .split_whitespace()
            .next()?
            .parse::<u64>()
            .ok()
    })?;
    kibibytes.checked_mul(1024)
}
