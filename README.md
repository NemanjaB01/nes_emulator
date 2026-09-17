# NES Emulator

A NES emulator written in Rust as a team project. I worked on the 6502 CPU implementation and testing.

## Included ROMs

- tetris.nes
- pacman.nes
- ice_climber.nes
- cyo.nes
- bb.nes

## How to run

With Rust and Cargo installed, run from the project folder:

```bash
cargo run --release -- tetris.nes
```

Replace `tetris.nes` with another ROM filename.

If you get a `No such audio device` error, run without sound:

```bash
SDL_AUDIODRIVER=dummy cargo run --release -- tetris.nes
```