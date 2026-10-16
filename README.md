# OpenSteelStudio

A native Linux app (Rust, GTK4 and libadwaita) for customizing SteelSeries keyboards: per-key RGB lighting and the little OLED screen.

> **Status: early.** Lighting, screen and typing effects work on the Apex 5. Expect rough edges.

## Supported devices

| Device | USB ID | Editor | Lighting output | Screen output |
| --- | --- | --- | --- | --- |
| Apex 5 | `1038:161c` | Yes | Yes | Yes |

More SteelSeries devices are planned.

## Features

**Lighting**
- Full Apex 5 layout, US (ANSI) or ISO
- Select keys by clicking, Shift-clicking or dragging a box; paint them, or pick a color from a key
- Swatches, a full color picker and hex input
- Presets: solid, rainbow, rainbow rows, random, gaming (WASD and arrows) and all off
- Animated effects: breathing, rainbow wave (five directions), color cycle and starlight, with adjustable speed
- Typing effects: keys light up or send out a ripple when pressed, in any color or rainbow

**Screen (128×40 OLED)**
- Pixel editor with pencil, eraser, invert and clear
- Text in any installed font, with alignment and pixel offsets
- Image import with fit, fill or stretch, dithering, invert and brightness
- Modes: your image, a clock (with seconds or date), scrolling text, or an animated GIF
- While typing: show your typing speed or a key counter for a few seconds. It never shows which keys you press, so passwords can't be read off the screen
- Live preview of exactly what the keyboard shows

**General**
- **Send to keyboard** shows everything on the keyboard live, and reconnects by itself when the keyboard is unplugged and plugged back in
- Typing effects react to typing anywhere while sending, and to keys typed in the app window otherwise
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

## Keyboard access

Linux only lets programs control the keyboard with permission. Install the included udev rule once (the app's "How to fix" button shows the same steps):

```sh
sudo install -m 644 data/70-open-steel-studio.rules /etc/udev/rules.d/
sudo udevadm control --reload-rules && sudo udevadm trigger --subsystem-match=hidraw
```

The rule only lets the logged-in user open the Apex 5's device files.

## Keyboard safety

OpenSteelStudio only ever sends three commands, all of which change temporary state: per-key colors, a screen image, and handing the lighting back to the keyboard. Nothing is written to the keyboard's flash or firmware, and unplugging the keyboard restores its defaults.

- Commands are only sent when the keyboard's HID descriptors exactly match the ones the protocol was verified against.
- Every packet is checked against those three commands and the exact report sizes before it is sent.
- Frames are only sent when they change, at most 30 times a second.
- Key presses are read from the keyboard only while a typing effect is on. Which keys were pressed stays in memory for 5 seconds at most and is never saved; the screen never shows which keys you press.

Animations are rendered by the app, so they stop when it closes. Static lighting stays on the keyboard until it is unplugged or goes to sleep.

Command-line tools:

```sh
open-steel-studio --probe              # read-only: find the keyboard and check its descriptors
open-steel-studio --restore-lighting   # hand the lighting back to the keyboard's own lighting
```

## Acknowledgements

Protocol details come from the open-source work of [OpenRGB](https://gitlab.com/CalcProgrammer1/OpenRGB) (per-key lighting) and [apex-tux](https://github.com/not-jan/apex-tux) (OLED screen).

OpenSteelStudio is an independent project, not affiliated with or endorsed by SteelSeries.

## License

OpenSteelStudio is free software: you can redistribute it and/or modify it under the terms of the [GNU General Public License](LICENSE) as published by the Free Software Foundation, either version 3 of the License, or (at your option) any later version.
