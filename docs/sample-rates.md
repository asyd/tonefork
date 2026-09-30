# Playing at a high sample rate on Linux

A DAC such as the ADI-2 accepts up to 768 kHz over USB, yet by default everything you play on a PipeWire desktop reaches it at **48 kHz**. This page explains why, how to change it, and how to check what the DAC really receives.

## Why you get 48 kHz

PipeWire runs a single clock for its whole graph (48 kHz by default) and **resamples every stream to that clock**. Asking for 192 kHz in an application is not enough: the application hands 192 kHz to PipeWire, PipeWire converts it to 48 kHz, and only then does the sound go to the DAC.

See the current settings (read-only):

```sh
pw-metadata -n settings | grep -E "clock\.(rate|allowed-rates|force-rate)"
```

On a default install: `clock.rate = 48000`, `clock.allowed-rates = [ 48000 ]`, `clock.force-rate = 0`.

## What the DAC can do

```sh
cat /proc/asound/cards                 # find your card number (here: card 4)
grep Rates /proc/asound/card4/stream0  # e.g. 44100 … 192000, 352800, 384000, 705600, 768000
```

## Check what is really received

This is the only reliable check: look at the ALSA hardware while something plays.

```sh
cat /proc/asound/card4/pcm0p/sub0/hw_params     # the "rate:" line; "closed" when idle
```

`tonefork` prints the same thing for you, 1.5 s after starting:

```
Stream : 192000 Hz, 2 ch, f32             <- what tonefork asked for
Hardware running now (what the DAC actually receives):
  DAC59920464 (card4): 48000 Hz, S32_LE   <- what reached the DAC
```

If the two rates differ, PipeWire resampled. (The ADI-2's own display also shows the incoming rate.)

## Three ways to change it

### 1. Force the graph rate, temporarily (simplest)

```sh
pw-metadata -n settings 0 clock.force-rate 192000   # the whole graph now runs at 192 kHz
# … play, check …
pw-metadata -n settings 0 clock.force-rate 0        # back to normal
```

Nothing is written to disk; the setting goes away when PipeWire restarts. **Every** application is then resampled to that rate, not only yours.

### 2. Let PipeWire follow the source (permanent)

Allow several rates, so the graph can switch to the rate a stream asks for. Create `~/.config/pipewire/pipewire.conf.d/10-rates.conf`:

```
context.properties = {
    default.clock.allowed-rates = [ 44100 48000 88200 96000 176400 192000 ]
}
```

Then restart the audio services (for example `systemctl --user restart pipewire pipewire-pulse wireplumber`). PipeWire ships an example limited to 96 kHz in `/usr/share/pipewire/pipewire.conf.avail/10-rates.conf`.

Whether the graph actually switches depends on the application and on other running streams: **always verify with `hw_params`**. If it does not follow, use method 1.

### 3. Bypass PipeWire

Play straight to the ALSA hardware device (`hw:…`) with a player in exclusive mode. The sound server must not hold the DAC, so you have to stop it or release the sink first. This is the most "pure" path but also the most awkward; it is mostly useful to prove that nothing in the chain touches the signal.

## With `tonefork`

The easy way: let `tonefork` force PipeWire's clock for the duration of the run and put it back afterwards.

```sh
tonefork --rate 192000 --force-clock --level -40 noise
```

What happens, in order: it reads the current `clock.force-rate`, sets it to the requested rate, waits a moment for the graph (and the DAC) to switch, plays, fades out on Ctrl-C, then **restores the previous value**. The "Hardware running now" line tells you whether it worked; if the hardware is still not at the requested rate, `tonefork` says so (another stream is probably pinning PipeWire's clock, see `pw-top`).

Notes:

- `--force-clock` needs `--rate`, and the `pw-metadata` command (part of PipeWire).
- The previous value is restored when `tonefork` exits normally or with Ctrl-C. If it is killed hard (`kill -9`, power loss of the session), reset it by hand: `pw-metadata -n settings 0 clock.force-rate 0`.
- Both the change and the restoration change the DAC's clock: expect a short interruption, possibly a click.

Without `--force-clock`, `--rate` only changes the rate of the stream handed to PipeWire; the hardware stays at 48 kHz, as the report shows. The manual equivalent of `--force-clock`:

```sh
pw-metadata -n settings 0 clock.force-rate 192000   # 1. make the graph run at the target rate
tonefork --rate 192000 noise                         # 2. ask for the same rate, check the report
pw-metadata -n settings 0 clock.force-rate 0        # 3. restore
```

## Things to know

- **No new information.** Playing 48 kHz material at 192 kHz only makes PipeWire's resampler work; nothing above 24 kHz appears. A higher rate is meaningful only for genuinely high-resolution files, or to exercise the DAC (its filters behave differently at other rates).
- **Nothing above ~20 kHz is audible**, so this has little value for judging an EQ by ear.
- **The DAC may click or mute briefly when the rate changes** (clock re-lock, output relays). Lower the DAC volume before switching, and do not switch with headphones on.
- **Load and glitches.** Very high rates raise CPU load and can cause dropouts on a busy machine. The ADI-2 handles them fine over USB 2 (768 kHz stereo is about 49 Mbit/s).
- **It affects the whole desktop** while forced: browser, notifications, video calls.
- Some PipeWire setups manage rates per device through WirePlumber rules; if a setting seems ignored, check for one in `~/.config/wireplumber/`.
