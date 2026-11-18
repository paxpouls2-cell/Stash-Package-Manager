# Stash

A small, modern package manager window for CachyOS / Arch (KDE Plasma). Search, install, remove and update, nothing else.

It is written in Rust (egui) but does no package logic of its own: every action is a normal `pacman` call, elevated through `pkexec` (the standard KDE password prompt). That keeps it as stable as pacman itself.

## Install

```bash
cd stash
./install.sh
```

If Rust is missing, the script installs it with `sudo pacman -S --needed rust base-devel`, builds the app (first build takes a few minutes) and adds a "Stash" entry to your app menu.

## Using it

- **Discover**: type a name, press Enter, click Install. Results come from the official repos, plus the AUR if `paru` or `yay` is installed (CachyOS ships paru). AUR installs open a Konsole window, since they build from source and may ask questions.
- **Installed**: packages you installed on purpose, with a filter box and Remove (also removes dependencies nothing else needs).
- **Updates**: lists pending updates and updates the whole system in one click.
- **Log**: the toggle in the bottom right shows live pacman output.

## Notes

- If an install fails with a 404/"failed retrieving file" error, your package databases are stale. Run **Update all** first, then retry. Stash deliberately never runs `pacman -Sy` on its own, because partial upgrades can break Arch systems.
- Pending updates are checked with `checkupdates` (package `pacman-contrib`) when available: `sudo pacman -S pacman-contrib`.
