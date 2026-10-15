# OpenSteelStudio

A native Linux app (Rust, GTK4 and libadwaita) for customizing SteelSeries keyboards: per-key RGB lighting and the little OLED screen.

> **Status: early preview.** The editor UI works, but it doesn't talk to the keyboard yet. Nothing is sent to your device.

## Supported devices

| Device | USB ID | Editor | Lighting output | Screen output |
| --- | --- | --- | --- | --- |
| Apex 5 | `1038:161c` | Yes | Planned | Planned |

More SteelSeries devices are planned.

## Features

**Lighting**
- Full Apex 5 layout, US (ANSI) or ISO
- Select keys by clicking, Shift-clicking or dragging a box; paint them, or pick a color from a key
- Swatches, a full color picker and hex input
- Presets: solid, rainbow, rainbow rows, random, gaming (WASD and arrows) and all off

**Screen (128×40 OLED)**
- Pixel editor with pencil, eraser, invert and clear
- Text in any installed font, with alignment and pixel offsets
- Image import with fit, fill or stretch, dithering, invert and brightness
- Actual-size preview

**General**
- Undo for each page (Ctrl+Z)
- Profiles saved as JSON (Ctrl+S / Ctrl+O)
- Restores your last session on startup

## Building

Requirements: Rust (stable), GTK 4.12 or newer, libadwaita 1.6 or newer.

```sh
cargo build --release
./target/release/open-steel-studio
```

On Arch-based systems the libraries come from the `gtk4` and `libadwaita` packages.

## Keyboard safety

The planned device support only uses commands that change temporary state: per-key colors, handing the lighting back to the keyboard, and screen images. Nothing is written to the keyboard's flash or firmware, and unplugging the keyboard restores its defaults. Commands are only sent when the device's HID descriptor matches the one the protocol was verified against.

## Acknowledgements

Protocol details come from the open-source work of [OpenRGB](https://gitlab.com/CalcProgrammer1/OpenRGB) (per-key lighting) and [apex-tux](https://github.com/not-jan/apex-tux) (OLED screen).

OpenSteelStudio is an independent project, not affiliated with or endorsed by SteelSeries.
