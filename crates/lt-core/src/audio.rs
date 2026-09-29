//! Lossless WAV to FLAC conversion for stems. FLAC holds exactly the same audio in roughly half the space;
//! every conversion is decoded again and compared sample by sample before the WAV is let go.

use anyhow::{Context, Result, anyhow, bail};
use flacenc::component::BitRepr;
use flacenc::error::Verify;
use std::path::Path;

/// Writes `dest` as a FLAC copy of the WAV at `src`. Returns false (and writes nothing) when the WAV
/// cannot be stored in FLAC without loss: floating-point or 32-bit samples.
pub fn wav_to_flac(src: &Path, dest: &Path) -> Result<bool> {
    let mut reader =
        hound::WavReader::open(src).with_context(|| format!("reading {}", src.display()))?;
    let spec = reader.spec();
    if spec.sample_format != hound::SampleFormat::Int || !(8..=24).contains(&spec.bits_per_sample) {
        return Ok(false);
    }
    let samples: Vec<i32> = reader
        .samples::<i32>()
        .collect::<std::result::Result<_, _>>()
        .with_context(|| format!("reading {}", src.display()))?;
    let (channels, bits, rate) = (
        spec.channels as usize,
        spec.bits_per_sample as usize,
        spec.sample_rate as usize,
    );
    let config = flacenc::config::Encoder::default()
        .into_verified()
        .map_err(|e| anyhow!("FLAC settings: {e:?}"))?;
    let source = flacenc::source::MemSource::from_samples(&samples, channels, bits, rate);
    let stream = flacenc::encode_with_fixed_block_size(&config, source, config.block_size)
        .map_err(|e| anyhow!("FLAC encoding failed: {e:?}"))?;
    let mut sink = flacenc::bitsink::ByteSink::new();
    stream
        .write(&mut sink)
        .map_err(|e| anyhow!("FLAC encoding failed: {e:?}"))?;

    let tmp = dest.with_extension("flac.part");
    std::fs::write(&tmp, sink.as_slice())?;
    let same = decodes_to(&tmp, &samples, channels, rate).unwrap_or(false);
    if !same {
        let _ = std::fs::remove_file(&tmp);
        bail!(
            "The FLAC copy of {} did not match the original, so the WAV was kept",
            src.display()
        );
    }
    std::fs::rename(&tmp, dest)?;
    Ok(true)
}

fn decodes_to(flac: &Path, want: &[i32], channels: usize, rate: usize) -> Result<bool> {
    let mut r = claxon::FlacReader::open(flac)?;
    let info = r.streaminfo();
    if info.channels as usize != channels || info.sample_rate as usize != rate {
        return Ok(false);
    }
    let mut n = 0;
    for s in r.samples() {
        if want.get(n) != Some(&s?) {
            return Ok(false);
        }
        n += 1;
    }
    Ok(n == want.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wav_becomes_identical_flac() {
        let d = tempfile::tempdir().unwrap();
        let wav = d.path().join("Vocals.wav");
        let spec = hound::WavSpec {
            channels: 2,
            sample_rate: 48000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut w = hound::WavWriter::create(&wav, spec).unwrap();
        for i in 0..48000i32 {
            let v = ((i as f32 * 0.05).sin() * 12000.0) as i16;
            w.write_sample(v).unwrap();
            w.write_sample(-v).unwrap();
        }
        w.finalize().unwrap();
        let flac = d.path().join("Vocals.flac");
        assert!(wav_to_flac(&wav, &flac).unwrap());
        assert!(std::fs::metadata(&flac).unwrap().len() < std::fs::metadata(&wav).unwrap().len());

        let float = d.path().join("Float.wav");
        let spec = hound::WavSpec {
            bits_per_sample: 32,
            sample_format: hound::SampleFormat::Float,
            ..spec
        };
        let mut w = hound::WavWriter::create(&float, spec).unwrap();
        w.write_sample(0.5f32).unwrap();
        w.write_sample(0.5f32).unwrap();
        w.finalize().unwrap();
        assert!(
            !wav_to_flac(&float, &d.path().join("Float.flac")).unwrap(),
            "float WAVs are left alone"
        );
    }
}
