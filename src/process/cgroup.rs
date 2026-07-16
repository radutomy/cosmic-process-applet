// SPDX-License-Identifier: MPL-2.0

#[derive(Clone, Debug, Default)]
pub(super) struct CgroupInfo {
    pub(super) ignored: bool,
    pub(super) user_unit: Option<String>,
    pub(super) app_scope_id: Option<String>,
}

impl CgroupInfo {
    pub(super) fn parse(contents: &str) -> Self {
        let Some(path) = contents
            .lines()
            .find_map(|line| line.split_once("::").map(|(_, path)| path))
        else {
            return Self::default();
        };

        let mut info = Self::default();
        let mut in_app_slice = false;
        for part in path.split('/').filter(|part| !part.is_empty()) {
            info.ignored |= part == "system.slice"
                || part == "init.scope"
                || (part.starts_with("session-") && part.ends_with(".scope"));
            if part == "app.slice" {
                in_app_slice = true;
            } else if in_app_slice {
                if part.ends_with(".service") {
                    info.user_unit = Some(part.to_string());
                }
                if let Some(id) = desktop_id_from_app_scope(part) {
                    info.app_scope_id = Some(id);
                }
            }
        }
        info
    }
}

pub(super) fn desktop_id_from_user_unit(unit: &str) -> Option<String> {
    let id = unit.strip_prefix("app-")?.strip_suffix(".service")?;
    Some(
        id.split_once('@')
            .map_or(id, |(id, _)| id)
            .replace("\\x2d", "-"),
    )
}

fn desktop_id_from_app_scope(scope: &str) -> Option<String> {
    let id = scope.strip_prefix("app-")?.strip_suffix(".scope")?;
    let id = id
        .strip_prefix("cosmic-")
        .unwrap_or(id)
        .replace("\\x2d", "-");
    Some(
        id.rsplit_once('-')
            .filter(|(_, suffix)| suffix.chars().all(|character| character.is_ascii_digit()))
            .map_or(id.as_str(), |(id, _)| id)
            .to_string(),
    )
}

pub(super) fn friendly_unit_name(unit: &str) -> String {
    unit.strip_suffix(".service")
        .or_else(|| unit.strip_suffix(".scope"))
        .unwrap_or(unit)
        .replace("\\x2d", "-")
        .replace(['-', '_'], " ")
}

pub(super) fn is_infrastructure_unit(unit: &str) -> bool {
    const UNITS: [&str; 13] = [
        "at-spi-dbus-bus.service",
        "com.system76.CosmicStatusNotifierWatcher.service",
        "dbus-broker.service",
        "dconf.service",
        "pipewire.service",
        "pipewire-pulse.service",
        "speech-dispatcher.service",
        "wireplumber.service",
        "xdg-desktop-portal.service",
        "xdg-desktop-portal-cosmic.service",
        "xdg-desktop-portal-gtk.service",
        "xdg-document-portal.service",
        "xdg-permission-store.service",
    ];
    UNITS.contains(&unit)
        || unit.starts_with("gvfs-")
        || unit.contains("org.a11y.atspi")
        || unit.contains("org.freedesktop.impl.portal")
}
