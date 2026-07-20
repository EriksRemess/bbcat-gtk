# bbcat GTK demo

`bbcat-gtk` is a small GTK4 demonstration app for the
[`bbcat`](https://crates.io/crates/bbcat) Rust library. It opens ANSI and BBS
art with `bbcat`, including 256-color ANSI animations, and displays the rendered
image in a GTK4 window.

This is currently a library demo, not a full-featured art viewer. It has a file
chooser, supports opening a file from the command line or desktop, and plays
animations at `bbcat`'s default speed. The window follows the rendered content
size up to the available monitor space. Artwork scales with the window while
preserving its aspect ratio, with black letterboxing around unused space.
Artwork taller or wider than the monitor instead remains at native size and
stays centered on any axis that fits; only the overflowing axes get scrollbars.
SAUCE titles, authors, and dates are shown in the window title when available.
There are no zoom, playback, editing, or configuration controls.

## Build and run

Rust and the GTK 4.8 or newer development files are required.

```sh
cargo run --release -- artwork.ans
```

The Open button can be used when no file is given.

## Install

The Makefile installs the release binary, `bbcat.desktop`, and the MIME
definitions used for file associations. To install them for the current user:

```sh
make install PREFIX="$HOME/.local"
```

Remove a Makefile-based user installation with the matching prefix:

```sh
make uninstall PREFIX="$HOME/.local"
```

The desktop entry associates the preview app with ANSI (`.ans`, `.asc`, `.diz`),
NFO, DarkDraw (`.ddw`), ArtWorx (`.adf`), RIPscrip (`.rip`), and XBin (`.xb`,
`.xbin`) artwork. The `update-mime-database` and `update-desktop-database`
commands are run after a direct install when they are available.

Packagers can stage an install without updating the host databases:

```sh
make install DESTDIR=/tmp/bbcat-gtk-package PREFIX=/usr
```

## Debian packages

Build a package for the current machine with:

```sh
make deb
```

The package is written to `dist/`. It includes the commit date and hash in its
version and derives its GTK runtime dependencies from the built binary.

GitHub Actions builds `amd64` and `arm64` packages on every push. Each build is
available as a workflow artifact; builds from `main` also replace the packages
on the rolling `tip` prerelease. The `arm64` package targets current 64-bit
Raspberry Pi OS on Raspberry Pi 4 and 5.

## How it works

Input is decoded with `bbcat::decode_with_options`. Static screens are encoded
with `bbcat`'s PNG renderer and loaded into a GTK4 texture. Animated documents
are rendered one frame at a time and scheduled through the GTK main loop, which
avoids retaining a texture for every frame.
