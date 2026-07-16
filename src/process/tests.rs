// SPDX-License-Identifier: MPL-2.0

use super::{
    ProcessIdentity, ProcessSnapshot, RawProcess, WorkloadKey,
    cgroup::{CgroupInfo, desktop_id_from_user_unit, friendly_unit_name, is_infrastructure_unit},
    classify_all,
    desktop::{DesktopApp, DesktopIndex, friendly_identity},
    identity_classification,
    platform::{open_verified_pidfd, process_start_ticks, proportional_memory_from_smaps_rollup},
    targets_for,
};

fn raw_process(pid: u32, parent: Option<u32>, name: &str) -> RawProcess {
    RawProcess {
        identity: ProcessIdentity {
            pid,
            start_ticks: u64::from(pid),
        },
        parent,
        name: name.to_string(),
        memory: 0,
        approximate_memory: false,
        cmd: None,
        exe: None,
        container: None,
        cgroup: CgroupInfo::default(),
        steam_game: None,
        tmux_server: false,
    }
}

fn desktop_index() -> DesktopIndex {
    DesktopIndex::new(vec![
        DesktopApp {
            id: "spotify.desktop".into(),
            name: "Spotify".into(),
            executable: "spotify".into(),
            no_display: false,
        },
        DesktopApp {
            id: "org.wezfurlong.wezterm.desktop".into(),
            name: "WezTerm".into(),
            executable: "wezterm-gui".into(),
            no_display: false,
        },
    ])
}

#[test]
fn executable_index_matches_nix_wrappers() {
    let mut process = raw_process(10, None, "spotify");
    process.exe = Some("/nix/store/example/share/spotify/.spotify-wrapped".into());
    let classification = identity_classification(&process, &desktop_index()).unwrap();
    assert_eq!(
        classification,
        WorkloadKey::Desktop("spotify.desktop".into())
    );
}

#[test]
fn spotify_children_inherit_one_application_boundary() {
    let mut root = raw_process(10, None, "spotify");
    root.exe = Some("/nix/store/example/.spotify-wrapped".into());
    let child = raw_process(11, Some(10), "spotify --type=renderer");
    let snapshot = ProcessSnapshot::from([(10, root), (11, child)]);
    let classifications = classify_all(&snapshot, &desktop_index());
    assert_eq!(classifications[&10], classifications[&11]);
}

#[test]
fn cgroup_is_parsed_once_into_structured_fields() {
    let parsed = CgroupInfo::parse(
        "0::/user.slice/user-1000.slice/user@1000.service/app.slice/app-org.wezfurlong.wezterm@autostart.service\n",
    );
    assert!(!parsed.ignored);
    assert_eq!(
        parsed.user_unit.as_deref(),
        Some("app-org.wezfurlong.wezterm@autostart.service")
    );

    let system = CgroupInfo::parse("0::/system.slice/NetworkManager.service\n");
    assert!(system.ignored);
    assert!(system.user_unit.is_none());
}

#[test]
fn app_scope_provides_a_desktop_hint() {
    let parsed = CgroupInfo::parse(
        "0::/user.slice/user-1000.slice/user@1000.service/app.slice/app-cosmic-org.wezfurlong.wezterm-1234.scope\n",
    );
    assert_eq!(
        parsed.app_scope_id.as_deref(),
        Some("org.wezfurlong.wezterm")
    );
}

#[test]
fn generic_launchers_are_not_indexed() {
    let index = DesktopIndex::new(vec![DesktopApp {
        id: "script.desktop".into(),
        name: "Script".into(),
        executable: "/usr/bin/python3".into(),
        no_display: false,
    }]);
    let mut process = raw_process(10, None, "python3");
    process.exe = Some("/usr/bin/python3".into());
    assert!(index.by_process_executable(&process).is_none());
}

#[test]
fn steam_game_launchers_do_not_claim_the_steam_client() {
    let index = DesktopIndex::new(vec![
        DesktopApp {
            id: "steam.desktop".into(),
            name: "Steam".into(),
            executable: "steam".into(),
            no_display: false,
        },
        DesktopApp {
            id: "danganronpa-v3.desktop".into(),
            name: "Danganronpa V3: Killing Harmony".into(),
            executable: "steam".into(),
            no_display: false,
        },
    ]);
    let mut process = raw_process(10, None, "steam");
    process.exe = Some("/home/user/.local/share/Steam/ubuntu12_32/steam".into());
    assert!(index.by_process_executable(&process).is_none());
}

#[test]
fn ambiguous_steam_executable_falls_back_to_scope_and_parent() {
    let index = DesktopIndex::new(vec![
        DesktopApp {
            id: "steam.desktop".into(),
            name: "Steam".into(),
            executable: "steam".into(),
            no_display: false,
        },
        DesktopApp {
            id: "danganronpa-v3.desktop".into(),
            name: "Danganronpa V3: Killing Harmony".into(),
            executable: "steam".into(),
            no_display: false,
        },
    ]);
    let mut root = raw_process(10, None, "bwrap");
    root.cgroup.app_scope_id = Some("steam".into());
    let mut steam = raw_process(11, Some(10), "steam");
    steam.exe = Some("/home/user/.local/share/Steam/ubuntu12_32/steam".into());
    let helper = raw_process(12, Some(11), "steamwebhelper");
    let snapshot = ProcessSnapshot::from([(10, root), (11, steam), (12, helper)]);
    let classifications = classify_all(&snapshot, &index);

    for classification in classifications.values() {
        assert_eq!(
            classification,
            &WorkloadKey::Desktop("steam.desktop".into())
        );
    }
}

#[test]
fn steam_games_are_separate_from_the_client() {
    let index = DesktopIndex::new(vec![DesktopApp {
        id: "steam.desktop".into(),
        name: "Steam".into(),
        executable: "steam".into(),
        no_display: false,
    }]);
    let mut steam = raw_process(10, None, "steam");
    steam.exe = Some("/home/user/.local/share/Steam/ubuntu12_32/steam".into());
    let mut reaper = raw_process(11, Some(10), "reaper");
    reaper.steam_game = Some(570);
    let game = raw_process(12, Some(11), "dota2");
    let mut overlay = raw_process(13, Some(10), "gameoverlayui");
    overlay.steam_game = reaper.steam_game;
    let snapshot = ProcessSnapshot::from([(10, steam), (11, reaper), (12, game), (13, overlay)]);
    let classifications = classify_all(&snapshot, &index);

    assert_eq!(
        classifications[&10],
        WorkloadKey::Desktop("steam.desktop".into())
    );
    for pid in [11, 12, 13] {
        assert_eq!(classifications[&pid], WorkloadKey::SteamGame(570));
    }
}

#[test]
fn current_process_has_a_start_time_and_pidfd() {
    let pid = std::process::id();
    let identity = ProcessIdentity {
        pid,
        start_ticks: process_start_ticks(pid).unwrap(),
    };
    assert!(open_verified_pidfd(identity).unwrap().is_some());
}

#[test]
fn stale_identity_is_not_accepted() {
    let pid = std::process::id();
    let identity = ProcessIdentity {
        pid,
        start_ticks: process_start_ticks(pid).unwrap() + 1,
    };
    assert!(open_verified_pidfd(identity).unwrap().is_none());
}

#[test]
fn service_names_are_human_readable() {
    assert_eq!(friendly_unit_name("app-foo_bar.service"), "app foo bar");
    assert_eq!(
        friendly_identity("com.example.My-App.desktop"),
        "com.example.My App"
    );
}

#[test]
fn desktop_id_is_recovered_from_autostart_unit() {
    assert_eq!(
        desktop_id_from_user_unit("app-org.wezfurlong.wezterm@autostart.service").as_deref(),
        Some("org.wezfurlong.wezterm")
    );
    assert_eq!(
        desktop_id_from_user_unit("app-com.example.foo\\x2dbar.service").as_deref(),
        Some("com.example.foo-bar")
    );
    assert!(desktop_id_from_user_unit("pipewire.service").is_none());
}

#[test]
fn session_cgroups_are_infrastructure() {
    assert!(CgroupInfo::parse("0::/user.slice/user-1000.slice/session-1.scope\n").ignored);
    assert!(
        CgroupInfo::parse("0::/user.slice/user-1000.slice/user@1000.service/init.scope\n").ignored
    );
    assert!(
        !CgroupInfo::parse(
            "0::/user.slice/user-1000.slice/user@1000.service/app.slice/app-firefox.scope\n"
        )
        .ignored
    );
}

#[test]
fn desktop_platform_units_are_infrastructure() {
    assert!(is_infrastructure_unit("pipewire.service"));
    assert!(is_infrastructure_unit(
        "dbus-:1.2-org.freedesktop.impl.portal.desktop.cosmic@0.service"
    ));
    assert!(is_infrastructure_unit(
        "gvfs-udisks2-volume-monitor.service"
    ));
    assert!(!is_infrastructure_unit(
        "app-org.wezfurlong.wezterm@autostart.service"
    ));
}

#[test]
fn proportional_memory_uses_pss_in_bytes() {
    let smaps = "Rss: 4096 kB\nPss: 1536 kB\nPss_Dirty: 512 kB\n";
    assert_eq!(
        proportional_memory_from_smaps_rollup(smaps),
        Some(1536 * 1024)
    );
}

#[test]
fn tmux_descendants_are_separate_from_terminal_in_a_shared_unit() {
    let unit = "app-org.wezfurlong.wezterm@autostart.service";
    let mut terminal = raw_process(10, None, "wezterm-gui");
    terminal.exe = Some("/usr/bin/wezterm-gui".into());
    terminal.cgroup.user_unit = Some(unit.into());
    let mut tmux = raw_process(11, None, "tmux");
    tmux.tmux_server = true;
    tmux.cgroup.user_unit = Some(unit.into());
    let mut shell = raw_process(12, Some(11), "bash");
    shell.cgroup.user_unit = Some(unit.into());
    let snapshot = ProcessSnapshot::from([(10, terminal), (11, tmux), (12, shell)]);
    let classifications = classify_all(&snapshot, &desktop_index());
    assert_ne!(classifications[&10], classifications[&11]);
    assert_eq!(classifications[&11], classifications[&12]);
    assert!(
        targets_for(&classifications[&10], &snapshot, &classifications)
            .1
            .is_empty()
    );
}

#[test]
fn unrelated_unknown_processes_remain_separate() {
    let first = raw_process(10, None, "first");
    let second = raw_process(11, Some(10), "second");
    let snapshot = ProcessSnapshot::from([(10, first), (11, second)]);
    let classifications = classify_all(&snapshot, &desktop_index());
    assert_ne!(classifications[&10], classifications[&11]);
}
