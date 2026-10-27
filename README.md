# omarchy-screensaver

The [Omarchy](https://omarchy.org) screensaver: a native Wayland overlay on every display that animates your branding text with [ttfx](https://github.com/omacom/ttfx) effects. A key press, click, scroll or mouse movement dismisses it.

## In Omarchy

Omarchy installs it as the `omarchy-screensaver` package, at `/usr/lib/omarchy-screensaver/omarchy-screensaver`, and starts it through `omarchy-launch-screensaver`, both when the session goes idle and from _System > Screensaver_. It:

- draws the text in `~/.config/omarchy/branding/screensaver.txt`, falling back to the bundled `assets/screensaver.txt`
- stays off while the `screensaver-off` toggle is set (`omarchy toggle screensaver`), or while the session is locked
- maps one layer surface per display under the namespace `org.omarchy.screensaver`, which is how Omarchy's tools find it
- runs one instance at a time, and on Hyprland hides the cursor while it is up and restores it when it exits
- fades out and exits on input, `SIGTERM`, `SIGINT`, `SIGHUP` or `SIGQUIT`

## Options

```bash
omarchy-launch-screensaver [force] [options]
```

- `force`: start even when the toggle is set or the session is locked
- `--input PATH`: draw this file instead of the branding text
- `--effect NAME`: pin one effect instead of picking at random; the names are ttfx's effects (`ttfx --help`)
- `--include-effects LIST`, `--exclude-effects LIST`: narrow the random pick
- `--fade N`: how long it fades out when dismissed, in milliseconds, default 200; it appears at once
- `--seed N`: repeat the same effect and animation
- `--headless --frames N --cols N --rows N`: render without a display, for tests

`--help` lists the rest.

## Build and test

Building needs Rust, `pkg-config` and libxkbcommon. Running it needs a compositor with `wlr-layer-shell`, such as Hyprland, and a monospace font: JetBrains Mono Nerd Font, or the closest monospace font `fc-match` finds.

```bash
make                                  # build/rust/release/omarchy-launch-screensaver
make check                            # the test suite; needs no compositor
make benchmark                        # render benchmark, checked byte-for-byte against a full redraw
make install PREFIX="$HOME/.local"    # as $PREFIX/bin/omarchy-launch-screensaver, outside Omarchy's package
```

Pushing a `v*` tag that matches the version in `Cargo.toml` publishes `x86_64` and `aarch64` release archives with SHA-256 checksums. [LEARNINGS.md](LEARNINGS.md) explains how rendering and presentation work.

## Credits

Written by [Tobi Lütke](https://github.com/tobi). Built on [ttfx](https://github.com/omacom/ttfx), [Smithay Client Toolkit](https://github.com/Smithay/client-toolkit) and [fontdue](https://github.com/mooman219/fontdue).

MIT; see [LICENSE](LICENSE).
