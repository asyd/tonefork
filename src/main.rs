//! tonegen: test signals to hear what EQ and Bass/Treble settings actually do.

mod signal;

use anyhow::{anyhow, bail, Context, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{Device, Error, FromSample, Host, HostId, SupportedStreamConfig, OutputCallbackInfo, SampleFormat, SizedSample, Stream, StreamConfig};
use signal::{Kind, Signal, OCTAVES};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

const DEFAULT_LEVEL_DB: f32 = -40.0;
/// Hard ceiling, in dBFS RMS. Protects ears and speakers.
const MAX_LEVEL_DB: f32 = -12.0;

const USAGE: &str = "\
tonegen - test signals to hear what EQ and Bass/Treble settings do

USAGE: tonegen [OPTIONS] <MODE>

MODES:
  list                  List the audio output devices
  tone <hz>             Continuous sine
  band <hz>             Continuous octave-wide pink-noise band (easier to judge than a sine)
  noise                 Continuous pink noise (flat on average: best for overall tonal balance)
  sweep                 Logarithmic sweep 20 Hz -> 20 kHz, repeated
  steps                 Walk through frequencies, one after the other

OPTIONS:
  --device <text>       Output device whose name contains <text> (default: system default)
  --rate <hz>           Request this sample rate for the stream (e.g. 96000, 192000). See docs/sample-rates.md
  --host <alsa|pulse>   Audio backend. Default: pulse (PipeWire/PulseAudio) when available, else alsa
  --level <dBFS>        RMS level, default -40, maximum -12
  --secs <n>            Seconds per step (steps, default 4) or per sweep (default 12)
  --freqs <a,b,c>       Frequencies for `steps` (default: the 10 octave bands 31.5 Hz..16 kHz)
  --noise               `steps` with band noise instead of sine tones
  -h, --help            This help

Ctrl-C fades out before stopping. Start with the DAC volume LOW, then raise it.";

enum Mode {
    List,
    Play(Kind),
}

struct Args {
    mode: Mode,
    device: Option<String>,
    host: Option<String>,
    rate: Option<u32>,
    level_db: f32,
}

fn parse() -> Result<Args> {
    let mut it = std::env::args().skip(1);
    let (mut device, mut host, mut rate, mut level_db) = (None, None, None, DEFAULT_LEVEL_DB);
    let (mut secs, mut freqs, mut noise) = (None, None::<Vec<f32>>, false);
    let mut positional = Vec::new();
    while let Some(a) = it.next() {
        match a.as_str() {
            "-h" | "--help" => {
                println!("{USAGE}");
                std::process::exit(0);
            }
            "--device" => device = Some(it.next().ok_or_else(|| anyhow!("--device needs a value"))?),
            "--host" => host = Some(it.next().ok_or_else(|| anyhow!("--host needs a value"))?),
            "--rate" => {
                let v = it.next().ok_or_else(|| anyhow!("--rate needs a value"))?;
                let r: u32 = v.parse().context("--rate must be an integer (Hz)")?;
                if !(8_000..=1_536_000).contains(&r) {
                    bail!("--rate must be between 8000 and 1536000 Hz");
                }
                rate = Some(r);
            }
            "--level" => {
                let v = it.next().ok_or_else(|| anyhow!("--level needs a value"))?;
                level_db = v.parse().context("--level must be a number (dBFS)")?;
            }
            "--secs" => {
                let v = it.next().ok_or_else(|| anyhow!("--secs needs a value"))?;
                secs = Some(v.parse::<f32>().context("--secs must be a number")?);
            }
            "--freqs" => {
                let v = it.next().ok_or_else(|| anyhow!("--freqs needs a value"))?;
                freqs = Some(v.split(',').map(|f| f.trim().parse::<f32>()).collect::<Result<_, _>>().context("--freqs: comma-separated numbers")?);
            }
            "--noise" => noise = true,
            _ => positional.push(a),
        }
    }
    if level_db > MAX_LEVEL_DB {
        bail!("--level {level_db} is above the {MAX_LEVEL_DB} dBFS safety ceiling");
    }
    let hz = |s: Option<&String>, usage: &str| -> Result<f32> {
        let f: f32 = s.ok_or_else(|| anyhow!("usage: tonegen {usage}"))?.parse().context("frequency in Hz expected")?;
        if !(20.0..=20000.0).contains(&f) {
            bail!("frequency must be between 20 and 20000 Hz");
        }
        Ok(f)
    };
    let mode = match positional.first().map(String::as_str) {
        Some("list") => Mode::List,
        Some("tone") => Mode::Play(Kind::Sine(hz(positional.get(1), "tone <hz>")?)),
        Some("band") => Mode::Play(Kind::Band(hz(positional.get(1), "band <hz>")?)),
        Some("noise") => Mode::Play(Kind::Pink),
        Some("sweep") => Mode::Play(Kind::Sweep(secs.unwrap_or(12.0).max(1.0))),
        Some("steps") => {
            let freqs = freqs.unwrap_or_else(|| OCTAVES.to_vec());
            if freqs.is_empty() || freqs.iter().any(|f| !(20.0..=20000.0).contains(f)) {
                bail!("--freqs: values must be between 20 and 20000 Hz");
            }
            Mode::Play(Kind::Steps { freqs, secs: secs.unwrap_or(4.0).max(0.5), noise })
        }
        _ => {
            eprintln!("{USAGE}");
            std::process::exit(2);
        }
    };
    Ok(Args { mode, device, host, rate, level_db })
}

fn label(d: &Device) -> String {
    let id = d.id().map(|i| i.to_string()).unwrap_or_else(|_| "?".into());
    match d.description() {
        Ok(desc) => format!("{id} ({desc})"),
        Err(_) => id,
    }
}

/// PipeWire/PulseAudio directly when possible: it avoids ALSA's `pulse` plugin, which can
/// report spurious I/O errors, and it lists every sink (even one the sound server holds).
fn get_host(name: Option<&str>) -> Result<Host> {
    let available = cpal::available_hosts();
    let id = match name.map(str::to_lowercase).as_deref() {
        Some("alsa") => HostId::Alsa,
        Some("pulse" | "pulseaudio" | "pipewire") => HostId::PulseAudio,
        Some(other) => bail!("unknown host {other:?} (alsa or pulse)"),
        None if available.contains(&HostId::PulseAudio) => HostId::PulseAudio,
        None => HostId::Alsa,
    };
    if !available.contains(&id) {
        bail!("audio backend {} is not available", id.name());
    }
    Ok(cpal::host_from_id(id)?)
}

fn find_device(host: &Host, pattern: Option<&str>) -> Result<Device> {
    let Some(p) = pattern else {
        return host.default_output_device().ok_or_else(|| anyhow!("no default output device"));
    };
    let p = p.to_lowercase();
    host.devices()?
        .filter(|d| d.default_output_config().is_ok())
        .find(|d| label(d).to_lowercase().contains(&p))
        .ok_or_else(|| anyhow!("no output device matches {p:?} (see `tonegen list`)"))
}

/// Picks the output configuration: the device default, or the requested sample rate
/// (stereo preferred, then f32 > i32 > i16).
fn pick_config(device: &Device, rate: Option<u32>) -> Result<SupportedStreamConfig> {
    let Some(rate) = rate else {
        return Ok(device.default_output_config()?);
    };
    let ranges: Vec<_> = device.supported_output_configs()?.collect();
    let rank = |c: &SupportedStreamConfig| {
        let fmt = match c.sample_format() {
            SampleFormat::F32 => 0,
            SampleFormat::I32 => 1,
            SampleFormat::I16 => 2,
            _ => return None,
        };
        Some((usize::from(c.channels() != 2), fmt))
    };
    ranges
        .iter()
        .filter_map(|r| r.clone().try_with_sample_rate(rate))
        .filter_map(|c| rank(&c).map(|k| (k, c)))
        .min_by_key(|(k, _)| *k)
        .map(|(_, c)| c)
        .ok_or_else(|| {
            let (lo, hi) = ranges.iter().fold((u32::MAX, 0), |(lo, hi), r| (lo.min(r.min_sample_rate()), hi.max(r.max_sample_rate())));
            anyhow!("this device cannot do {rate} Hz (it reports {lo}..{hi} Hz)")
        })
}

/// What the ALSA hardware is actually running at right now (Linux `/proc/asound`).
/// This is the real answer: a sound server may resample a stream to its own clock.
fn hardware_report() -> Vec<String> {
    let mut lines = Vec::new();
    let Ok(cards) = std::fs::read_dir("/proc/asound") else { return lines };
    let mut cards: Vec<_> = cards.filter_map(|e| e.ok()).map(|e| e.path()).collect();
    cards.sort();
    for card in cards {
        let name = card.file_name().and_then(|n| n.to_str()).unwrap_or("").to_string();
        if !name.starts_with("card") || name[4..].parse::<u32>().is_err() {
            continue;
        }
        let id = std::fs::read_to_string(card.join("id")).unwrap_or_default();
        let Ok(pcms) = std::fs::read_dir(&card) else { continue };
        let mut pcms: Vec<_> = pcms.filter_map(|e| e.ok()).map(|e| e.path()).collect();
        pcms.sort();
        for pcm in pcms {
            let pn = pcm.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if !(pn.starts_with("pcm") && pn.ends_with('p')) {
                continue;
            }
            let Ok(hw) = std::fs::read_to_string(pcm.join("sub0/hw_params")) else { continue };
            let field = |k: &str| hw.lines().find_map(|l| l.strip_prefix(k)).map(|v| v.trim().to_string());
            if let (Some(rate), Some(fmt)) = (field("rate:"), field("format:")) {
                let rate = rate.split_whitespace().next().unwrap_or("?");
                lines.push(format!("{} ({}): {rate} Hz, {fmt}", id.trim(), name));
            }
        }
    }
    lines
}

fn build<T>(device: &Device, config: StreamConfig, mut sig: Signal, errors: Arc<AtomicUsize>) -> Result<Stream>
where
    T: SizedSample + FromSample<f32>,
{
    let channels = config.channels as usize;
    let stream = device.build_output_stream(
        config,
        move |data: &mut [T], _: &OutputCallbackInfo| {
            for frame in data.chunks_mut(channels) {
                let v: T = T::from_sample(sig.next());
                frame.iter_mut().for_each(|s| *s = v);
            }
        },
        move |e: Error| {
            // Report the first error only: they tend to repeat many times per second.
            if errors.fetch_add(1, Ordering::Relaxed) == 0 {
                eprintln!("audio error: {e} (further errors are counted, not printed)");
            }
        },
        None,
    )?;
    Ok(stream)
}

fn main() -> Result<()> {
    let args = parse()?;
    let host = get_host(args.host.as_deref())?;
    let Mode::Play(kind) = args.mode else {
        println!("Backend: {}", host.id().name());
        for d in host.devices()?.filter(|d| d.default_output_config().is_ok()) {
            println!("{}", label(&d));
        }
        return Ok(());
    };

    let device = find_device(&host, args.device.as_deref())?;
    let supported = pick_config(&device, args.rate)?;
    let config: StreamConfig = supported.clone().into();
    let sr = config.sample_rate as f32;

    let (stop, done) = (Arc::new(AtomicBool::new(false)), Arc::new(AtomicBool::new(false)));
    let steps = match &kind {
        Kind::Steps { freqs, .. } => Some(freqs.clone()),
        _ => None,
    };
    let sig = Signal::new(kind, sr, args.level_db, stop.clone(), done.clone());
    let position = sig.position.clone();

    let errors = Arc::new(AtomicUsize::new(0));
    let stream = match supported.sample_format() {
        SampleFormat::F32 => build::<f32>(&device, config, sig, errors.clone())?,
        SampleFormat::I16 => build::<i16>(&device, config, sig, errors.clone())?,
        SampleFormat::I32 => build::<i32>(&device, config, sig, errors.clone())?,
        f => bail!("unsupported sample format {f}"),
    };

    ctrlc::set_handler({
        let stop = stop.clone();
        move || stop.store(true, Ordering::Relaxed)
    })?;

    println!("Backend: {}", host.id().name());
    println!("Device : {}", label(&device));
    println!("Stream : {} Hz, {} ch, {}", supported.sample_rate(), supported.channels(), supported.sample_format());
    println!("Level  : {:.0} dBFS RMS (max {MAX_LEVEL_DB:.0}). Start with the DAC volume low!", args.level_db);
    println!("Ctrl-C to stop (fades out).\n");
    stream.play()?;
    std::thread::sleep(Duration::from_millis(1500));
    let hw = hardware_report();
    if hw.is_empty() {
        println!("Hardware: (no running ALSA playback found)");
    } else {
        println!("Hardware running now (what the DAC actually receives):");
        hw.iter().for_each(|l| println!("  {l}"));
    }
    println!();

    let mut last = usize::MAX;
    while !done.load(Ordering::Relaxed) {
        if let Some(f) = &steps {
            let p = position.load(Ordering::Relaxed);
            if p != last {
                last = p;
                println!("  {:>8.1} Hz", f[p]);
            }
        }
        std::thread::sleep(Duration::from_millis(40));
    }
    std::thread::sleep(Duration::from_millis(150)); // let the last buffer drain
    let n = errors.load(Ordering::Relaxed);
    if n > 0 {
        eprintln!("{n} audio error(s) during playback");
    }
    Ok(())
}
