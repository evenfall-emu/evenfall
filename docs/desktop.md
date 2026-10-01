# Desktop player

`evenfall-gba-desktop` plays a GBA ROM in a window, with the keyboard and the default sound device.
It draws through the shared [presenter](design/presentation.md), so it scales the picture as the web player does.
It is a thin host around [`Session`](../systems/gba/src/session.rs): it paces frames against the wall clock, feeds sound to the device and writes the battery save.
The source is [`desktop/gba/`](../desktop/gba/src/main.rs).

## Build and run

```sh
cargo run --release -p evenfall-gba-desktop -- game.gba
```

On Linux the build needs ALSA's development files and `pkg-config` (`libasound2-dev pkg-config` on Debian and Ubuntu).
At run time it needs a Vulkan or OpenGL driver, and on X11 `libxkbcommon-x11`.
wgpu's `WGPU_BACKEND` variable picks the graphics backend, such as `WGPU_BACKEND=gl`.

| Option                             | Effect                                                 |
| ---------------------------------- | ------------------------------------------------------ |
| `--bios <path>`                    | Boot through a 16 KiB BIOS image instead of HLE        |
| `--save-file <path>`               | Keep the battery save there instead of next to the ROM |
| `--preset pixel` or `vanilla-plus` | The scaling preset to start with, Pixel by default     |
| `--integer-scale`                  | Start with whole-number scaling                        |

A ZIP holding one ROM works like the ROM itself.

## Keys

| Key               | Action                            |
| ----------------- | --------------------------------- |
| Arrows            | D-pad                             |
| X / Z             | A / B                             |
| Enter / Backspace | Start / Select                    |
| A / S             | L / R                             |
| Space             | Pause or resume                   |
| Tab (hold)        | Run 8 frames per frame, silent    |
| F2                | Switch between Pixel and Vanilla+ |
| F3                | Whole-number scaling on or off    |
| F11               | Fullscreen                        |

Losing window focus releases every key.

## How it runs

A frame is due every 280,896 master cycles of wall-clock time, about 16.74 ms.
The player runs due frames back to back to make up a short stall.
More than 200 ms behind, it restarts its schedule from the clock instead, as the browser's Worker does.
Each frame runs to an exact cycle target, so an instruction that overshoots one frame's end shortens the next instead of shifting every later frame.

Sound goes through a queue of at most a tenth of a second.
The emulated clock and the sound device's clock drift apart slowly, so the queue drops its oldest samples when full and plays silence when empty.
Without a sound device the game plays silently.

The battery save is written next to the ROM as `<name>.sav`, the same file the [CLI](cli.md) uses, whenever it changed: every five seconds while playing, and on exit.
Writes replace the file atomically.
The real-time clock follows the computer's clock for cartridges that have one.

## Known limits

- No menus, save states, rewind, gamepads or cheats yet, and settings aren't stored.
- Fast-forward mutes sound.
- Only Linux has been run, on Mesa's software rasterizers under Xvfb.
  macOS and Windows haven't been run.
