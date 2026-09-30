# tonegen

A tiny test-signal generator to **hear what your EQ and Bass/Treble settings do**, even without a trained ear.
Written in Rust (`cpal`), made as a companion to [rmediy-rs](https://github.com/asyd/rmediy-rs).

Every mode is scaled to the same **RMS** level, so switching between a sine, a band of noise and pink noise does not change the perceived loudness much. A 1 s fade-in and a fade-out on Ctrl-C avoid clicks.

## Modes

```sh
tonegen list              # output devices
tonegen noise             # pink noise: best to judge the overall tonal balance
tonegen band 85           # octave-wide noise band around 85 Hz (the default Bass frequency)
tonegen band 6500         # ... around 6.5 kHz (the default Treble frequency)
tonegen tone 1000         # continuous sine
tonegen sweep --secs 12   # log sweep 20 Hz -> 20 kHz, repeated
tonegen steps             # walks through the 10 octave bands, 4 s each, printing the frequency
tonegen steps --noise --freqs 85,250,1000,6500,10000
```

Options: `--level <dBFS>` (RMS, default **-40**, hard maximum **-12**), `--secs`, `--freqs`, `--noise`, `--device <text>`, `--host <alsa|pulse>`, `--rate <Hz>`.

## A listening recipe

1. Start with the DAC volume **low**. The default level is deliberately quiet.
2. Play `tonegen noise` and toggle **EQ Enable** / **B/T Enable** in rmediy-rs: pink noise sounds "flat" and any boost or cut is easy to notice.
3. To hear one setting precisely, play `tonegen band <freq>` at the frequency you are changing and move only that band's gain: you should hear that band get louder or quieter.
4. Use `tonegen steps --noise` to walk across the spectrum and hear which regions a setting affects.

Why noise bands rather than sines? Low and very high sine tones are hard to hear at low level (our ears are far less sensitive there), and sines excite room resonances. A band of noise is easier to judge.

## Audio backend and device (Linux)

By default `tonegen` talks **directly to PipeWire / PulseAudio** (`--host pulse`), which lists every sink by name, even the one your desktop is already using, and avoids ALSA's `pulse` plugin. Use `--host alsa` to go through ALSA instead (then a device held by the sound server is busy and will not be listed).

```sh
tonegen list                        # sinks, by name
tonegen --device ADI noise          # the sink whose name contains "ADI" (e.g. an ADI-2 DAC)
tonegen --host alsa --device hw:CARD=DAC noise
```

Without `--device`, the default sink of your session is used. The signal goes through the sound server's volume as well as the DAC's own volume.

Audio errors, if any, are reported once and counted (they can repeat many times per second with some ALSA setups).

## Sample rate

`--rate 192000` asks for a high sample rate, but a PipeWire desktop resamples everything to its own clock (48 kHz by default). `tonegen` prints the rate the hardware is **really** running at, so you can see whether the request went through. How to make it do so: [docs/sample-rates.md](docs/sample-rates.md). The path the audio takes: [docs/audio-path.md](docs/audio-path.md).

## Safety

Test signals can be loud and sustained. `--level` refuses anything above -12 dBFS RMS, but the real loudness also depends on the sink volume and on the DAC volume (and on what is connected). Start low.

## License

MIT, see [LICENSE](LICENSE).
