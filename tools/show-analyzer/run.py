"""Makes shows for a list of songs with a few workers sharing the GPU. See README.md."""
import argparse
import json
import multiprocessing as mp
import os
import queue
import time
import traceback

# Demucs's model is kept next to the analyzer (models/), not in the user's general cache folder.
os.environ.setdefault("TORCH_HOME", os.path.join(os.path.dirname(os.path.abspath(__file__)), "models"))


def worker(q, done, args):
    import imageio_ffmpeg
    from analyze import Separator, analyze
    sep = Separator()
    ffmpeg = imageio_ffmpeg.get_ffmpeg_exe()
    while True:
        item = q.get()
        if item is None:
            return
        uid, rel = item
        out = os.path.join(args.out, f"{uid}.json")
        t0 = time.time()
        try:
            show = analyze(uid, rel if os.path.isabs(rel) else os.path.join(args.music, rel), ffmpeg, sep)
            tmp = out + ".part"
            with open(tmp, "w") as f:
                json.dump(show, f, separators=(",", ":"))
            os.replace(tmp, out)
            done.put(("ok", uid, rel, round(time.time() - t0, 1), os.path.getsize(out)))
        except Exception as e:
            done.put(("failed", uid, rel, round(time.time() - t0, 1), f"{type(e).__name__}: {e}\n{traceback.format_exc(limit=3)}"))


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--music", default="", help="music root for relative paths in the list (lists made by the app use full paths)")
    ap.add_argument("--list", required=True)
    ap.add_argument("--out", required=True)
    ap.add_argument("--workers", type=int, default=1, help="songs at once; 2-3 can help on a large NVIDIA GPU")
    ap.add_argument("--limit", type=int, default=0)
    args = ap.parse_args()
    os.makedirs(args.out, exist_ok=True)
    todo = []
    for line in open(args.list, encoding="utf-8"):
        uid, rel = line.rstrip("\n").split("\t", 1)
        if not os.path.exists(os.path.join(args.out, f"{uid}.json")):
            todo.append((uid, rel))
    if args.limit:
        todo = todo[: args.limit]
    print(f"{len(todo)} songs to do with {args.workers} workers", flush=True)
    ctx = mp.get_context("spawn")
    q, done = ctx.Queue(), ctx.Queue()
    procs = [ctx.Process(target=worker, args=(q, done, args)) for _ in range(args.workers)]
    for p in procs:
        p.start()
    for item in todo:
        q.put(item)
    for _ in procs:
        q.put(None)
    t0, ok, failed = time.time(), 0, 0
    with open(os.path.join(args.out, "_log.tsv"), "a", encoding="utf-8") as log:
        for i in range(len(todo)):
            while True:
                try:
                    state, uid, rel, secs, info = done.get(timeout=10)
                    break
                except queue.Empty:
                    if not any(p.is_alive() for p in procs):
                        print("The analyzer stopped unexpectedly (see the error above). Run setup again, then try once more.", flush=True)
                        raise SystemExit(1)
            ok += state == "ok"
            failed += state == "failed"
            log.write(f"{state}\t{uid}\t{secs}\t{rel}\t{info if state == 'failed' else info}\n".replace("\n\t", " "))
            log.flush()
            rate = (time.time() - t0) / (i + 1)
            print(f"[{i + 1}/{len(todo)}] {state} {secs}s {rel[-60:]}  (ok {ok}, failed {failed}, ~{rate * (len(todo) - i - 1) / 60:.0f} min left)", flush=True)
            if state == "failed":
                print(info, flush=True)
    for p in procs:
        p.join()
    print(f"done: ok {ok}, failed {failed}, {(time.time() - t0) / 60:.1f} min", flush=True)


if __name__ == "__main__":
    main()
