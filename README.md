# tonefork

A tiny test-signal generator to **hear what your EQ and Bass/Treble settings actually do**, even without a trained ear.

It plays steady tones, bands of noise, pink noise and sweeps at a controlled, matched loudness, so you can switch a setting on and off and *hear* the difference. Made as a companion to [rmediy-rs](https://github.com/asyd/rmediy-rs) (a terminal remote control for the RME ADI-2 DAC), but it works with any audio device.

It can also show, and force, the sample rate your hardware really receives: handy to check what a DAC gets through PipeWire (see [Sample rate](#sample-rate)).

## Download

A Linux amd64 binary of the latest commit on `main` is published as the [**latest** pre-release](https://github.com/asyd/tonefork/releases/tag/latest). It is rebuilt at every push (not a versioned release), on Debian 12, so it runs on Debian 12+, Ubuntu 22.04+ and most recent distributions. It needs the ALSA runtime library (`libasound2` on Debian/Ubuntu).

```sh
gh release download latest --repo asyd/tonefork    # or download the archive from the release page
sha256sum -c tonefork-linux-amd64.tar.gz.sha256
tar xzf tonefork-linux-amd64.tar.gz && ./tonefork list
```

Or build it yourself, as described below.

## Quick start

```sh
git clone https://github.com/asyd/tonefork && cd tonefork
cargo build --release          # binary: target/release/tonefork

tonefork list                   # output devices
tonefork noise                  # pink noise, -40 dBFS (quiet), Ctrl-C to stop
```

Start with your DAC / amplifier volume **low**, then raise it.

Requirements: Rust 1.85+, and on Debian/Ubuntu `libasound2-dev` and `pkg-config` to build. Linux with PipeWire or PulseAudio is the tested setup (`--force-clock` also needs the `pw-metadata` command).

## Modes

| Command | What it plays |
|---|---|
| `tonefork noise` | Pink noise. Sounds "flat", so any boost or cut is easy to notice: best for the overall tonal balance. |
| `tonefork band 85` | An octave-wide band of noise around 85 Hz. Easier to judge than a sine. |
| `tonefork tone 1000` | A continuous sine. |
| `tonefork sweep` | A logarithmic sweep from 20 Hz to 20 kHz, repeated. |
| `tonefork steps` | Walks through the ten octave bands (31.5 Hz to 16 kHz), 4 s each, printing the frequency. |
| `tonefork list` | Lists the output devices. |

Every mode is scaled to the same **RMS** level, so switching between them does not change the perceived loudness much. Sound fades in over 1 s and fades out on Ctrl-C, so there are no clicks.

## Options

| Option | Meaning |
|---|---|
| `--level <dBFS>` | RMS level. Default **-40**, hard maximum **-12**. |
| `--secs <n>` | Seconds per step (`steps`, default 4) or per sweep (default 12). |
| `--freqs <a,b,c>` | Frequencies for `steps`, e.g. `--freqs 85,250,1000,6500`. |
| `--noise` | `steps` with band noise instead of sine tones. |
| `--device <text>` | Output device whose name contains `<text>`. Default: the session's default sink. |
| `--host <alsa\|pulse>` | Audio backend. Default: PipeWire/PulseAudio when available, else ALSA. |
| `--rate <Hz>` | Request a sample rate for the stream. |
| `--force-clock` | With `--rate`: set PipeWire's clock to that rate while playing, then restore it. |

## A listening recipe

1. Start with the volume low. The default level is deliberately quiet.
2. Play `tonefork noise` and switch your EQ or Bass/Treble on and off: you should hear the tonal balance change.
3. To hear one setting precisely, play `tonefork band <freq>` at the frequency you are adjusting and move only that band's gain.
4. Use `tonefork steps --noise` to walk across the spectrum and find which regions a setting affects.

Why noise rather than sines? Low and very high sine tones are hard to hear at low level (the ear is much less sensitive there), and sines excite room resonances. A band of noise is easier to judge.

## Sample rate

A PipeWire desktop resamples everything to its own clock (48 kHz by default), whatever an application asks for. `tonefork` prints the rate the hardware is **really** running at, 1.5 s after starting:

```
Stream : 192000 Hz, 2 ch, f32             <- what tonefork asked for
Hardware running now (what the DAC actually receives):
  DAC59920464 (card4): 192000 Hz, S32_LE  <- what reached the DAC
```

```sh
tonefork --rate 192000 --force-clock noise
```

This forces PipeWire's clock for the duration of the run and restores it afterwards. It affects the whole desktop and the DAC may click when the rate changes, so lower the volume first. Details, alternatives and caveats: [docs/sample-rates.md](docs/sample-rates.md).

## Documentation

- [docs/audio-path.md](docs/audio-path.md): the layers between `tonefork` and the speakers (cpal, PipeWire, ALSA, USB, the DAC), with a diagram.
- [docs/sample-rates.md](docs/sample-rates.md): playing at a high sample rate on Linux, and checking what the DAC receives.

## Safety

Test signals can be loud and sustained. `--level` refuses anything above -12 dBFS RMS, but the real loudness also depends on the sound server's volume, the DAC's volume and what is connected. Start low, and do not use headphones while changing the sample rate.

## License

MIT, see [LICENSE](LICENSE).
