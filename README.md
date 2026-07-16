# COSMIC Process Applet

A small COSMIC panel applet that displays the seven current-user applications
or process groups using the most proportional memory. Desktop applications,
containers, user services, and process trees are grouped when they can be
identified confidently; unknown processes remain individual rows. Each row has
a button that immediately kills every process in that workload.

Process discovery runs only while the popup is open and never blocks the UI.
Kill targets are reopened from a fresh process snapshot and addressed through
Linux pidfds, preventing an exited process's PID from being reused to kill an
unrelated process. A user service is stopped only when every process in it
belongs to the selected workload; shared services (such as a terminal hosting
a detached tmux server) are left intact.

Application memory is calculated from Linux proportional set size (PSS), which
divides shared pages between the processes using them instead of counting the
same shared pages once per process. Detached tmux servers and their sessions are
kept separate from the terminal application that attached to them. Steam games
are identified by their App ID and grouped separately from the Steam client, so
killing a game leaves Steam running.

The project is based on
[`pop-os/cosmic-applet-template`](https://github.com/pop-os/cosmic-applet-template).
Its process refresh model follows COSMIC Monitor's `sysinfo`-based approach,
but it runs independently because COSMIC Monitor does not expose a process-data
API.

## Development

```sh
nix develop
just run
just check
nix build
```

The flake exposes a default development shell, `packages.<system>.default`,
`packages.<system>.cosmic-process-applet`, `apps.<system>.default`, and a
default overlay. To consume the package from another flake:

```nix
inputs.cosmic-process-applet.url = "github:radutomy/cosmic-process-applet";
inputs.cosmic-process-applet.inputs.nixpkgs.follows = "nixpkgs";

# In a Home Manager or NixOS module:
home.packages = [ inputs.cosmic-process-applet.packages.${pkgs.system}.default ];
```

After installation, add **Process Killer** from COSMIC Settings' panel applet
list.
