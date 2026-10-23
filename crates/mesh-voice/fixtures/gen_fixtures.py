#!/usr/bin/env python3
"""Regenerates the mesh-voice fixture clips (DESIGN §13.1: 16 kHz WAVs, <= 2 MB in total).

Run from anywhere on macOS (needs `say`; Python stdlib only):

    python3 -I crates/mesh-voice/fixtures/gen_fixtures.py

Outputs, next to this script:
  speech.wav     ES/EN phrases from the macOS system voices (synthetic, licence-free),
                 shaped by a 150 Hz 2nd-order high-pass (a helmet boom-mic response: the
                 compact TTS voices carry more energy below 200 Hz than real close-talk
                 speech), separated by digital silence, normalised to -22 dBFS RMS over speech.
  speech.labels  one "start_ms end_ms" line per spoken phrase (VAD ground truth).
  wind.wav       gusty low-frequency noise (energy mostly below 200 Hz), seeded.
  engine.wav     motorcycle-like engine: ~45 Hz firing harmonics plus noise, seeded.
"""

import math
import os
import random
import struct
import subprocess
import sys
import tempfile
import wave

RATE = 16000
HERE = os.path.dirname(os.path.abspath(__file__))

PHRASES = [
    ("Samantha", "Left turn ahead, stay in the right lane."),
    ("Mónica", "Paramos a repostar en la próxima gasolinera."),
    ("Kathy", "I can see you in my mirror, keep going."),
    ("Flo (Spanish (Mexico))", "Cuidado, hay grava en la curva."),
    ("Fred", "Fuel stop in ten kilometers."),
    ("Daniel", "Group is complete, we are moving out."),
]
SILENCES_MS = [1000, 1500, 1800, 1200, 2000, 1400, 1500]


def say(voice, text):
    with tempfile.TemporaryDirectory() as d:
        path = os.path.join(d, "p.wav")
        subprocess.run(
            ["say", "-v", voice, "-o", path, "--file-format=WAVE", "--data-format=LEI16@16000", text],
            check=True,
        )
        with wave.open(path) as w:
            assert w.getframerate() == RATE and w.getnchannels() == 1 and w.getsampwidth() == 2
            raw = w.readframes(w.getnframes())
    pcm = list(struct.unpack("<%dh" % (len(raw) // 2), raw))
    # Trim leading/trailing digital silence so the labels are tight.
    thr = 200
    first = next((i for i, s in enumerate(pcm) if abs(s) > thr), 0)
    last = len(pcm) - next((i for i, s in enumerate(reversed(pcm)) if abs(s) > thr), 0)
    return pcm[first:last]


def write_wav(name, pcm):
    with wave.open(os.path.join(HERE, name), "wb") as w:
        w.setnchannels(1)
        w.setsampwidth(2)
        w.setframerate(RATE)
        w.writeframes(struct.pack("<%dh" % len(pcm), *[max(-32768, min(32767, int(round(s)))) for s in pcm]))


def highpass(pcm, fc):
    w = 2 * math.pi * fc / RATE
    alpha = math.sin(w) / (2 * math.sqrt(0.5))
    c = math.cos(w)
    a0 = 1 + alpha
    b = [(1 + c) / 2 / a0, -(1 + c) / a0, (1 + c) / 2 / a0]
    a = [-2 * c / a0, (1 - alpha) / a0]
    x1 = x2 = y1 = y2 = 0.0
    out = []
    for x in pcm:
        y = b[0] * x + b[1] * x1 + b[2] * x2 - a[0] * y1 - a[1] * y2
        x2, x1, y2, y1 = x1, x, y1, y
        out.append(y)
    return out


def speech():
    out, labels = [], []
    out += [0] * (SILENCES_MS[0] * RATE // 1000)
    for i, (voice, text) in enumerate(PHRASES):
        p = highpass(say(voice, text), 150.0)
        start = len(out)
        out += p
        labels.append((start * 1000 // RATE, len(out) * 1000 // RATE))
        out += [0] * (SILENCES_MS[i + 1] * RATE // 1000)
    speech_samples = [s for a, b in labels for s in out[a * RATE // 1000 : b * RATE // 1000]]
    rms = math.sqrt(sum(s * s for s in speech_samples) / len(speech_samples))
    gain = (32768 * 10 ** (-22 / 20)) / rms
    write_wav("speech.wav", [s * gain for s in out])
    with open(os.path.join(HERE, "speech.labels"), "w") as f:
        for a, b in labels:
            f.write(f"{a} {b}\n")


def one_pole(x, prev, a):
    return a * prev + (1 - a) * x


def wind(seconds=8.0):
    rnd = random.Random(0x57494E44)
    n = int(seconds * RATE)
    out, lp1, lp2, gust = [], 0.0, 0.0, 0.0
    a = math.exp(-2 * math.pi * 120 / RATE)
    for i in range(n):
        lp1 = one_pole(rnd.gauss(0, 1), lp1, a)
        lp2 = one_pole(lp1, lp2, a)
        if i % 160 == 0:
            gust = 0.97 * gust + 0.03 * rnd.uniform(0.2, 1.6)
        out.append(lp2 * gust)
    peak = max(abs(s) for s in out)
    write_wav("wind.wav", [s / peak * 32768 * 10 ** (-3 / 20) for s in out])


def engine(seconds=8.0):
    rnd = random.Random(0x454E47)
    n = int(seconds * RATE)
    out, phase, noise = [], 0.0, 0.0
    for i in range(n):
        t = i / RATE
        f0 = 45.0 + 4.0 * math.sin(2 * math.pi * 0.3 * t)
        phase += 2 * math.pi * f0 / RATE
        s = sum((1.0 / k) * math.sin(k * phase + 0.7 * k) for k in range(1, 24))
        noise = one_pole(rnd.gauss(0, 1), noise, 0.6)
        out.append(s + 0.25 * noise)
    peak = max(abs(s) for s in out)
    write_wav("engine.wav", [s / peak * 32768 * 10 ** (-6 / 20) for s in out])


if __name__ == "__main__":
    if sys.platform != "darwin":
        sys.exit("needs macOS `say`")
    speech()
    wind()
    engine()
