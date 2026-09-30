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

Options: `--level <dBFS>` (RMS, default **-40**, hard maximum **-12**), `--secs`, `--freqs`, `--noise`, `--device <text>`.

## A listening recipe

1. Start with the DAC volume **low**. The default level is deliberately quiet.
2. Play `tonegen noise` and toggle **EQ Enable** / **B/T Enable** in rmediy-rs: pink noise sounds "flat" and any boost or cut is easy to notice.
3. To hear one setting precisely, play `tonegen band <freq>` at the frequency you are changing and move only that band's gain: you should hear that band get louder or quieter.
4. Use `tonegen steps --noise` to walk across the spectrum and hear which regions a setting affects.

Why noise bands rather than sines? Low and very high sine tones are hard to hear at low level (our ears are far less sensitive there), and sines excite room resonances. A band of noise is easier to judge.

## Audio device on Linux (PipeWire / PulseAudio)

If your sound server already uses the DAC (as it usually does), direct ALSA access is busy and the DAC will not show up in `tonegen list`. Play through the sound server instead: the default device follows your default sink. To pick another sink:

```sh
PULSE_SINK=<sink name> tonegen --device pulse noise     # names: pactl list short sinks
```

Note that the signal goes through the sound server's volume as well as the DAC's own volume.

## Safety

Test signals can be loud and sustained. `--level` refuses anything above -12 dBFS RMS, but the real loudness also depends on the sink volume and on the DAC volume (and on what is connected). Start low.

## License

MIT, see [LICENSE](LICENSE).
