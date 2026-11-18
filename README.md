# Stash

A small, modern package manager window for CachyOS / Arch Linux. Search, install, remove and update, nothing else.

It is written in Rust ([egui](https://github.com/emilk/egui)) but does no package logic of its own: every action is a normal `pacman` call, elevated through `pkexec` (your desktop's standard password prompt). That keeps it as stable as pacman itself.

Works on any desktop (KDE Plasma, GNOME, XFCE, ...) on Arch-based systems with a polkit agent. It needs `pacman`, so it does not run on Debian, Ubuntu or Fedora.

## Features

- **Recommended apps:** the Discover tab opens with a hand-picked list of popular apps (browsers, media, creative, office, games) you can install with one click.
- **Search:** official repos, plus the AUR if `paru` or `yay` is installed.
- **Installed:** the packages you installed on purpose, with a filter box and Remove (also removes dependencies nothing else needs).
- **Updates:** see what is pending and update the whole system in one click.
- **Live log:** a toggle in the bottom right shows pacman's output as it runs.
- **Your terminal:** AUR packages build from source and may ask questions, so they open in your default terminal (via `xdg-terminal-exec`, KDE's setting, `$TERMINAL`, or the first common terminal found).

## Install

```bash
git clone https://github.com/paxpouls2-cell/Stash-Package-Manager.git
cd Stash-Package-Manager
./install.sh
```

If Rust is missing, the script installs it with `sudo pacman -S --needed rust base-devel`, builds the app (the first build takes a few minutes) and adds a "Stash" entry to your app menu. The binary goes to `~/.local/bin/stash`.

## Update to a newer version

```bash
cd Stash-Package-Manager
git pull
./uninstall.sh
./install.sh
```

## Uninstall

```bash
./uninstall.sh
```

You can free the build files with `rm -rf target`.

## Notes

- If an install fails with a 404 or "failed retrieving file" error, your package databases are stale. Run **Update all** first, then retry. Stash deliberately never runs `pacman -Sy` on its own, because partial upgrades can break Arch systems.
- Pending updates are checked with `checkupdates` (package `pacman-contrib`) when available: `sudo pacman -S pacman-contrib`.
- A recommended app that is not in your enabled repos (for example `steam` needs the `multilib` repo, which CachyOS enables by default) will fail with "target not found" in the log.

## License

[MIT](LICENSE)
