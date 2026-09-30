//! tonegen: test signals to hear what EQ and Bass/Treble settings actually do.

mod signal;

use anyhow::{anyhow, bail, Context, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{Device, Error, FromSample, OutputCallbackInfo, SampleFormat, SizedSample, Stream, StreamConfig};
use signal::{Kind, Signal, OCTAVES};
use std::sync::atomic::{AtomicBool, Ordering};
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
    level_db: f32,
}

fn parse() -> Result<Args> {
    let mut it = std::env::args().skip(1);
    let (mut device, mut level_db) = (None, DEFAULT_LEVEL_DB);
    let (mut secs, mut freqs, mut noise) = (None, None::<Vec<f32>>, false);
    let mut positional = Vec::new();
    while let Some(a) = it.next() {
        match a.as_str() {
            "-h" | "--help" => {
                println!("{USAGE}");
                std::process::exit(0);
            }
            "--device" => device = Some(it.next().ok_or_else(|| anyhow!("--device needs a value"))?),
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
    Ok(Args { mode, device, level_db })
}

fn label(d: &Device) -> String {
    let id = d.id().map(|i| i.to_string()).unwrap_or_else(|_| "?".into());
    match d.description() {
        Ok(desc) => format!("{id} ({desc})"),
        Err(_) => id,
    }
}

fn find_device(pattern: Option<&str>) -> Result<Device> {
    let host = cpal::default_host();
    let Some(p) = pattern else {
        return host.default_output_device().ok_or_else(|| anyhow!("no default output device"));
    };
    let p = p.to_lowercase();
    host.devices()?
        .filter(|d| d.default_output_config().is_ok())
        .find(|d| label(d).to_lowercase().contains(&p))
        .ok_or_else(|| anyhow!("no output device matches {p:?} (see `tonegen list`)"))
}

fn build<T>(device: &Device, config: StreamConfig, mut sig: Signal) -> Result<Stream>
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
        |e: Error| eprintln!("audio error: {e}"),
        None,
    )?;
    Ok(stream)
}

fn main() -> Result<()> {
    let args = parse()?;
    let Mode::Play(kind) = args.mode else {
        let host = cpal::default_host();
        for d in host.devices()?.filter(|d| d.default_output_config().is_ok()) {
            println!("{}", label(&d));
        }
        return Ok(());
    };

    let device = find_device(args.device.as_deref())?;
    let supported = device.default_output_config()?;
    let config: StreamConfig = supported.clone().into();
    let sr = config.sample_rate as f32;

    let (stop, done) = (Arc::new(AtomicBool::new(false)), Arc::new(AtomicBool::new(false)));
    let steps = match &kind {
        Kind::Steps { freqs, .. } => Some(freqs.clone()),
        _ => None,
    };
    let sig = Signal::new(kind, sr, args.level_db, stop.clone(), done.clone());
    let position = sig.position.clone();

    let stream = match supported.sample_format() {
        SampleFormat::F32 => build::<f32>(&device, config, sig)?,
        SampleFormat::I16 => build::<i16>(&device, config, sig)?,
        SampleFormat::I32 => build::<i32>(&device, config, sig)?,
        f => bail!("unsupported sample format {f}"),
    };

    ctrlc::set_handler({
        let stop = stop.clone();
        move || stop.store(true, Ordering::Relaxed)
    })?;

    println!("Device : {}", label(&device));
    println!("Level  : {:.0} dBFS RMS (max {MAX_LEVEL_DB:.0}). Start with the DAC volume low!", args.level_db);
    println!("Ctrl-C to stop (fades out).\n");
    stream.play()?;

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
    Ok(())
}
