// SPDX-License-Identifier: MPL-2.0

use std::{
    collections::HashMap,
    env, fs,
    path::{Path, PathBuf},
    sync::LazyLock,
};

use sysinfo::Process;

pub(super) fn steam_game(process: &Process) -> Option<u32> {
    let app_id = process
        .environ()
        .iter()
        .filter_map(|entry| entry.to_str())
        .find_map(|entry| {
            let (key, value) = entry.split_once('=')?;
            matches!(
                key,
                "SteamAppId" | "SteamGameId" | "STEAM_COMPAT_APP_ID" | "SteamOverlayGameId"
            )
            .then(|| value.parse().ok())
            .flatten()
        })
        .or_else(|| app_id_from_command(process))?;
    (app_id != 0).then_some(app_id)
}

pub(super) fn game_name(app_id: u32) -> String {
    CATALOG
        .get(&app_id)
        .cloned()
        .unwrap_or_else(|| format!("Steam game {app_id}"))
}

fn app_id_from_command(process: &Process) -> Option<u32> {
    let command = process.cmd();
    let executable = command.first().and_then(|path| Path::new(path).file_name());
    let is_reaper = executable.is_some_and(|name| name == "reaper")
        && command.iter().any(|argument| argument == "SteamLaunch");
    let is_overlay = executable.is_some_and(|name| name == "gameoverlayui");
    if !is_reaper && !is_overlay {
        return None;
    }
    command.iter().enumerate().find_map(|(index, argument)| {
        let argument = argument.to_str()?;
        argument
            .strip_prefix("AppId=")
            .or_else(|| argument.strip_prefix("-gameid="))
            .or_else(|| {
                (argument == "-gameid")
                    .then(|| command.get(index + 1)?.to_str())
                    .flatten()
            })?
            .parse()
            .ok()
    })
}

static CATALOG: LazyLock<HashMap<u32, String>> = LazyLock::new(load_catalog);

fn load_catalog() -> HashMap<u32, String> {
    let mut steamapps = default_steamapps_paths();
    for path in steamapps.clone() {
        if let Ok(contents) = fs::read_to_string(path.join("libraryfolders.vdf")) {
            steamapps.extend(
                contents
                    .lines()
                    .filter_map(|line| vdf_value(line, "path"))
                    .map(|path| PathBuf::from(path).join("steamapps")),
            );
        }
    }

    steamapps.sort_unstable();
    steamapps.dedup();
    let mut catalog = HashMap::new();
    for path in steamapps {
        catalog.extend(
            fs::read_dir(path)
                .into_iter()
                .flatten()
                .flatten()
                .filter_map(|entry| read_manifest(&entry.path())),
        );
    }
    catalog
}

fn read_manifest(path: &Path) -> Option<(u32, String)> {
    let app_id = manifest_app_id(path)?;
    let contents = fs::read_to_string(path).ok()?;
    let name = contents.lines().find_map(|line| vdf_value(line, "name"))?;
    Some((app_id, name))
}

fn default_steamapps_paths() -> Vec<PathBuf> {
    let home = env::var_os("HOME").map(PathBuf::from);
    [
        env::var_os("XDG_DATA_HOME").map(|path| PathBuf::from(path).join("Steam/steamapps")),
        home.as_ref()
            .map(|path| path.join(".local/share/Steam/steamapps")),
        home.as_ref()
            .map(|path| path.join(".steam/steam/steamapps")),
        home.map(|path| path.join(".var/app/com.valvesoftware.Steam/.local/share/Steam/steamapps")),
    ]
    .into_iter()
    .flatten()
    .collect()
}

fn manifest_app_id(path: &Path) -> Option<u32> {
    path.file_name()?
        .to_str()?
        .strip_prefix("appmanifest_")?
        .strip_suffix(".acf")?
        .parse()
        .ok()
}

fn vdf_value(line: &str, wanted_key: &str) -> Option<String> {
    let mut fields = line.split('"');
    fields.next()?;
    let key = fields.next()?;
    fields.next()?;
    let value = fields.next()?;
    (key.eq_ignore_ascii_case(wanted_key)).then(|| value.replace("\\\\", "\\"))
}

#[cfg(test)]
mod tests {
    use super::{manifest_app_id, vdf_value};
    use std::path::Path;

    #[test]
    fn steam_manifest_metadata_is_parsed() {
        assert_eq!(manifest_app_id(Path::new("appmanifest_570.acf")), Some(570));
        assert_eq!(
            vdf_value("\t\"name\"\t\t\"Dota 2\"", "name").as_deref(),
            Some("Dota 2")
        );
        assert_eq!(
            vdf_value("\t\"path\"\t\t\"/mnt/games\"", "path").as_deref(),
            Some("/mnt/games")
        );
    }
}
