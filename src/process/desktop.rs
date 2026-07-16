// SPDX-License-Identifier: MPL-2.0

use std::{collections::HashMap, ffi::OsStr, path::Path, sync::LazyLock};

use freedesktop_desktop_entry::{DesktopEntry, Iter, default_paths, get_languages_from_env};

use super::{RawProcess, WorkloadInfo, WorkloadKey, cgroup::is_infrastructure_unit};

#[derive(Debug)]
pub(super) struct DesktopApp {
    pub(super) id: String,
    pub(super) name: String,
    pub(super) executable: String,
    pub(super) no_display: bool,
}

#[derive(Default)]
pub(super) struct DesktopIndex {
    apps: HashMap<String, DesktopApp>,
    executables: HashMap<String, Option<String>>,
}

impl DesktopIndex {
    pub(super) fn new(apps: Vec<DesktopApp>) -> Self {
        let mut index = Self::default();
        for app in apps {
            let id = app
                .id
                .strip_suffix(".desktop")
                .unwrap_or(&app.id)
                .to_string();
            if !app.no_display
                && let Some(executable) = normalized_executable_name(&app.executable)
                && !is_generic_launcher(OsStr::new(&executable))
            {
                index
                    .executables
                    .entry(executable)
                    .and_modify(|owner| {
                        if owner.as_deref().is_some_and(|owner| owner != id) {
                            *owner = None;
                        }
                    })
                    .or_insert_with(|| Some(id.clone()));
            }
            index.apps.entry(id).or_insert(app);
        }
        index
    }

    fn load() -> Self {
        let locales = get_languages_from_env();
        let apps = Iter::new(default_paths())
            .filter_map(|path| DesktopEntry::from_path(path, Some(&locales)).ok())
            .filter_map(|entry| {
                Some(DesktopApp {
                    executable: entry.parse_exec().ok()?.first()?.clone(),
                    id: entry.id().to_string(),
                    name: entry
                        .full_name(&locales)
                        .unwrap_or_else(|| entry.id().into())
                        .to_string(),
                    no_display: entry.no_display(),
                })
            })
            .collect();
        Self::new(apps)
    }

    pub(super) fn by_process_executable(&self, process: &RawProcess) -> Option<&DesktopApp> {
        process
            .cmd
            .iter()
            .chain(process.exe.iter())
            .filter_map(|path| normalized_executable_name(path))
            .find_map(|name| {
                self.executables
                    .get(&name)
                    .and_then(Option::as_deref)
                    .and_then(|id| self.apps.get(id))
            })
    }

    pub(super) fn by_desktop_id(&self, id: &str) -> Option<&DesktopApp> {
        let id = id.strip_suffix(".desktop").unwrap_or(id);
        self.apps.get(id)
    }

    pub(super) fn by_scope_hint(&self, hint: &str) -> Option<&DesktopApp> {
        self.by_desktop_id(hint).or_else(|| {
            self.apps
                .iter()
                .filter(|(id, _)| hint.starts_with(&format!("{id}-")))
                .max_by_key(|(id, _)| id.len())
                .map(|(_, app)| app)
        })
    }
}

pub(super) fn friendly_identity(identity: &str) -> String {
    Path::new(identity)
        .file_stem()
        .and_then(OsStr::to_str)
        .unwrap_or(identity)
        .replace(['-', '_'], " ")
}

pub(super) fn is_infrastructure(workload: &WorkloadInfo) -> bool {
    match &workload.key {
        WorkloadKey::Desktop(id) => matches!(
            id.strip_suffix(".desktop").unwrap_or(id),
            "com.system76.CosmicLauncher" | "com.system76.CosmicWorkspaces"
        ),
        WorkloadKey::UserUnit(unit) => is_infrastructure_unit(unit),
        WorkloadKey::SteamGame(_) => false,
        WorkloadKey::Process(_) | WorkloadKey::ProcessTree(_) => {
            let name = workload.name.trim_start_matches('.');
            let name = name.strip_suffix("-wrapped").unwrap_or(name);
            name.starts_with("cosmic-")
                || [
                    "pop-launcher",
                    "cosmic-toplevel",
                    "crashhelper",
                    "chrome_crashpad_handler",
                ]
                .contains(&name)
        }
        WorkloadKey::Container(_) => false,
    }
}

fn normalized_executable_name(executable: &str) -> Option<String> {
    let name = Path::new(executable).file_name()?.to_str()?;
    let name = name.strip_prefix('.').unwrap_or(name);
    Some(name.strip_suffix("-wrapped").unwrap_or(name).to_string())
}

fn is_generic_launcher(name: &OsStr) -> bool {
    const LAUNCHERS: [&str; 10] = [
        "env", "sh", "bash", "dash", "zsh", "fish", "python", "python3", "node", "java",
    ];
    name.to_str().is_some_and(|name| LAUNCHERS.contains(&name))
}

pub(super) static INDEX: LazyLock<DesktopIndex> = LazyLock::new(DesktopIndex::load);
