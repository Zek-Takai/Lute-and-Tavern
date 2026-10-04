"""One song → one show. See README.md.

Show format (FORMAT = 1), JSON, times in seconds:
  uid, duration, tempo, beats[], downbeats[]
  sections: [{start, end, energy 0..1 (within the song), group}]   group = same kind of part (chorus = chorus)
  hits: {kick, snare, hat}: [[time, strength 0..1], ...]           from the drums part
  bass, other: {notes: [[time, strength]], env}                    note attacks and level
  vocal: {phrases: [[start, end]], env, pitch}                      pitch: MIDI note per 0.1 s, 0 = silent
  mix: {env}
  env = {rate: 20, v: [0..255]}: level 20 times a second, 255 = the part's loudest moment in this song
  key: {tonic, mode}
"""
import json
import subprocess

import librosa
import numpy as np

FORMAT = 1
SR = 44100        # decode and separation rate
AR = 22050        # analysis rate
HOP = 512
ENV_RATE = 20


def load(path: str, ffmpeg: str) -> np.ndarray:
    """Stereo float32 (2, n) at SR, decoded by ffmpeg (any format)."""
    raw = subprocess.run([ffmpeg, "-v", "error", "-i", path, "-f", "f32le", "-ac", "2", "-ar", str(SR), "-"],
                         capture_output=True, check=True).stdout
    return np.frombuffer(raw, dtype=np.float32).reshape(-1, 2).T.copy()


class Separator:
    def __init__(self, device: str = "cuda"):
        import torch
        from demucs.pretrained import get_model
        self.torch = torch
        self.model = get_model("htdemucs").to(device).eval()
        self.device = device

    def __call__(self, wav: np.ndarray) -> dict:
        from demucs.apply import apply_model
        t = self.torch
        ref = wav.mean(0)
        mean, std = float(ref.mean()), float(ref.std()) or 1.0
        x = t.from_numpy((wav - mean) / std)[None].to(self.device)
        with t.no_grad():
            out = apply_model(self.model, x, split=True, overlap=0.25, progress=False)[0]
        out = (out * std + mean).cpu().numpy()
        return dict(zip(self.model.sources, out))  # drums, bass, other, vocals


def mono(x: np.ndarray) -> np.ndarray:
    return librosa.resample(x.mean(0) if x.ndim == 2 else x, orig_sr=SR, target_sr=AR, res_type="soxr_hq")


def env(y: np.ndarray) -> dict:
    rms = librosa.feature.rms(y=y, frame_length=2048, hop_length=HOP)[0]
    per = AR / HOP / ENV_RATE
    n = int(len(rms) / per)
    v = np.array([rms[int(i * per):int((i + 1) * per)].mean() for i in range(n)]) if n else np.zeros(0)
    top = v.max() if len(v) and v.max() > 0 else 1.0
    return {"rate": ENV_RATE, "v": np.clip(np.round(v / top * 255), 0, 255).astype(int).tolist()}


def onsets(envelope: np.ndarray, wait_s: float, keep=None, delta: float = 0.1) -> list:
    """Peaks in an onset envelope as [[time, strength 0..1]]. `keep(frame)` can drop peaks."""
    if not len(envelope) or envelope.max() <= 0:
        return []
    e = envelope / envelope.max()
    peaks = librosa.onset.onset_detect(onset_envelope=e, sr=AR, hop_length=HOP, units="frames",
                                       wait=max(1, int(wait_s * AR / HOP)), pre_avg=10, post_avg=10, pre_max=3, post_max=3, delta=delta)
    return [[round(float(librosa.frames_to_time(f, sr=AR, hop_length=HOP)), 3), round(float(e[f]), 2)] for f in peaks if keep is None or keep(f)]


def band_flux(S: np.ndarray, freqs: np.ndarray, lo: float, hi: float) -> np.ndarray:
    b = S[(freqs >= lo) & (freqs < hi)]
    lb = np.log1p(b * 100)
    return np.maximum(0, np.diff(lb, axis=1, prepend=lb[:, :1])).sum(0)


def drums(y: np.ndarray) -> dict:
    S = np.abs(librosa.stft(y, n_fft=2048, hop_length=HOP))
    f = librosa.fft_frequencies(sr=AR, n_fft=2048)
    level = lambda lo, hi: (lambda b: b / (np.median(b) + 1e-9))(S[(f >= lo) & (f < hi)].sum(0))
    low, mid = level(30, 150), level(1500, 5000)
    # each hit goes to the drum whose band jumped most at that moment (bleed from the other drum doesn't count)
    return {
        "kick": onsets(band_flux(S, f, 30, 150), 0.1, keep=lambda fr: low[fr] >= mid[fr] * 0.6),
        "snare": onsets(band_flux(S, f, 1500, 5000) + 0.5 * band_flux(S, f, 150, 300), 0.1, keep=lambda fr: mid[fr] >= low[fr] * 0.6),
        "hat": onsets(band_flux(S, f, 6000, 11000), 0.05),
    }


def notes(y: np.ndarray) -> list:
    return onsets(librosa.onset.onset_strength(y=y, sr=AR, hop_length=HOP), 0.08)


def vocal(y: np.ndarray) -> dict:
    e = env(y)
    v = np.array(e["v"], dtype=float)
    on = v > max(30, np.percentile(v, 60) * 0.5) if len(v) else v.astype(bool)
    phrases, start, last_on = [], None, None
    for i, x in enumerate(on):
        t = i / ENV_RATE
        if x:
            if start is None:
                start = t
            last_on = t
        elif start is not None and t - last_on > 0.35:
            if last_on - start >= 0.4:
                phrases.append([round(start, 2), round(last_on + 1 / ENV_RATE, 2)])
            start = None
    if start is not None and last_on - start >= 0.4:
        phrases.append([round(start, 2), round(last_on + 1 / ENV_RATE, 2)])
    hop10 = AR // 10
    f0 = librosa.yin(y, fmin=80, fmax=1000, sr=AR, frame_length=4096, hop_length=hop10)
    level = librosa.feature.rms(y=y, frame_length=4096, hop_length=hop10)[0]
    loud = level > (level.max() * 0.12 if level.max() > 0 else 1)
    midi = [int(round(librosa.hz_to_midi(h))) if ok else 0 for h, ok in zip(f0, loud[:len(f0)])]
    return {"phrases": phrases, "env": e, "pitch": {"rate": 10, "midi": midi}}


MAJOR = np.array([6.35, 2.23, 3.48, 2.33, 4.38, 4.09, 2.52, 5.19, 2.39, 3.66, 2.29, 2.88])
MINOR = np.array([6.33, 2.68, 3.52, 5.38, 2.60, 3.53, 2.54, 4.75, 3.98, 2.69, 3.34, 3.17])
NAMES = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"]


def key(chroma: np.ndarray) -> dict:
    c = chroma.mean(1)
    best = max(((np.corrcoef(np.roll(p, k), c)[0, 1], k, m) for m, p in (("major", MAJOR), ("minor", MINOR)) for k in range(12)))
    return {"tonic": NAMES[best[1]], "mode": best[2]}


def sections(mix: np.ndarray, harmonic: np.ndarray, beats_f: np.ndarray, duration: float) -> list:
    chroma = librosa.feature.chroma_cqt(y=harmonic, sr=AR, hop_length=HOP)
    mfcc = librosa.feature.mfcc(y=mix, sr=AR, hop_length=HOP, n_mfcc=13)
    rms = librosa.feature.rms(y=mix, hop_length=HOP)[0]
    if len(beats_f) < 8:
        return [{"start": 0.0, "end": round(duration, 2), "energy": 0.5, "group": 0}], chroma
    sync = lambda F: librosa.util.sync(F, beats_f, aggregate=np.median)
    feats = np.vstack([librosa.util.normalize(sync(chroma), axis=0), librosa.util.normalize(sync(mfcc), axis=0)])
    k = int(np.clip(round(duration / 22), 3, 12))
    k = min(k, feats.shape[1] - 1)
    bounds = librosa.segment.agglomerative(feats, k)
    times = [0.0] + [float(librosa.frames_to_time(beats_f[b - 1] if b > 0 else 0, sr=AR, hop_length=HOP)) for b in bounds[1:]] + [duration]
    out, means = [], []
    lo, hi = np.percentile(rms, 5), np.percentile(rms, 98)
    for a, b in zip(times[:-1], times[1:]):
        if b - a < 1:
            continue
        fa, fb = librosa.time_to_frames([a, b], sr=AR, hop_length=HOP)
        seg = rms[fa:max(fa + 1, fb)]
        energy = float(np.clip((seg.mean() - lo) / max(hi - lo, 1e-9), 0, 1))
        ca, cb = np.searchsorted(librosa.frames_to_time(beats_f, sr=AR, hop_length=HOP), [a, b])
        m = feats[:, ca:max(ca + 1, cb)].mean(1)
        m = m / (np.linalg.norm(m) or 1)
        group = next((g for g, mm in enumerate(means) if float(m @ mm) > 0.92), None)
        if group is None:
            means.append(m)
            group = len(means) - 1
        out.append({"start": round(a, 2), "end": round(b, 2), "energy": round(energy, 2), "group": group})
    return out, chroma


def downbeats(beat_times: np.ndarray, kick: list) -> list:
    if len(beat_times) < 8 or not kick:
        return []
    kt = np.array([k[0] for k in kick])
    ks = np.array([k[1] for k in kick])
    def score(phase):
        s = 0.0
        for t in beat_times[phase::4]:
            near = np.abs(kt - t) < 0.07
            s += ks[near].max() if near.any() else 0
        return s
    p = max(range(4), key=score)
    return [round(float(t), 3) for t in beat_times[p::4]]


def analyze(uid: str, path: str, ffmpeg: str, separate) -> dict:
    wav = load(path, ffmpeg)
    duration = wav.shape[1] / SR
    parts = separate(wav)
    mix = mono(wav)
    d, b, o, v = (mono(parts[k]) for k in ("drums", "bass", "other", "vocals"))
    tempo, beats_f = librosa.beat.beat_track(y=mix, sr=AR, hop_length=HOP)
    beat_times = librosa.frames_to_time(beats_f, sr=AR, hop_length=HOP)
    hits = drums(d)
    secs, chroma = sections(mix, o + v, beats_f, duration)
    return {
        "format": FORMAT, "uid": uid, "duration": round(duration, 2),
        "tempo": round(float(np.atleast_1d(tempo)[0]), 1),
        "beats": [round(float(t), 3) for t in beat_times],
        "downbeats": downbeats(beat_times, hits["kick"]),
        "sections": secs,
        "hits": hits,
        "bass": {"notes": notes(b), "env": env(b)},
        "other": {"notes": notes(o), "env": env(o)},
        "vocal": vocal(v),
        "mix": {"env": env(mix)},
        "key": key(chroma),
        "made": {"by": "lute-show-analyzer", "separation": "htdemucs"},
    }
