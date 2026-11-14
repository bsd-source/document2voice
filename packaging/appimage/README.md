# AppImage packaging

Static files used by the root `./build-appimage.sh` script:

- `AppRun` — startup script (sets paths for the programs and voice models packed inside)
- `document2voice.desktop` — desktop menu / launcher info for Linux
- `collect-licenses.py` — gathers license texts and `THIRD-PARTY-NOTICES.md` for everything bundled
- `locks/` — conda lockfiles pinning Poppler and the espeak-ng build tools (see below)

The Kokoro model URL and SHA-256 live in `packaging/models.env`, shared by
`setup.sh` and `build-appimage.sh`.

## Build

```bash
./setup.sh                 # once
./build-appimage.sh        # build release apps and pack them → dist/
```

Output: `dist/Document2Voice-Kokoro-*-x86_64.AppImage`

Helper tools (ffmpeg, speech engine, PDF tools) are downloaded or built into
`target/appimage-cache/` — they are **not** copied from your system install.
Shared libraries are stored as real files so the AppImage still works when
copied to an external drive (no broken shortcuts/links).

Settings live under XDG config (`~/.config/document2voice/`), not next to the AppImage.

## Pronunciation

Text is turned into speech sounds (phonemes) by **espeak-ng**, after the speech
sanitizer. espeak keeps punctuation, so Kokoro's pauses and intonation follow
commas, full stops and questions.

Heteronyms espeak gets wrong are fixed first with a part-of-speech tagger
(`crates/core/data/pos-tagger/`): past-tense "read", and — from an audit of 38
common heteronyms in verb and noun use, where espeak was wrong on 20 of 76 —
contract, progress, subject, conflict, desert, insult, increase, excuse, close,
wind, wound, separate, estimate, moderate (verbs), produce, import, associate,
minute, does (nouns/adjectives). The audit runs as a test (`heteronym_audit_espeak`).
Words that differ only by meaning (lead, bow, row, bass, "a minute walk" vs
"a minute detail") cannot be told apart by grammar.

No network is used at any point.

## Chapters

Chapters are detected when a document is loaded; the Log lists the titles on Open
and again when Start begins:

| Source | Chapters from |
|---|---|
| EPUB | table of contents (EPUB 3 nav, or EPUB 2 `toc.ncx`); `<h1>`/`<h2>` if there is none |
| PDF | bookmarks (outline), top level |
| DOCX / ODT | Heading 1 (or Heading 2) paragraphs / top outline level |
| TXT, MOBI/AZW | headings such as `CHAPTER I.`, `Chapter 12`, `BOOK II`, `Part One`, `Letter 4`; a contents list (3+ headings in a row) is ignored |

Chapters shorter than ~40 words are merged into the one before. Text before the
first chapter becomes "Opening". With a page range, the first chapter keeps the
title of the chapter the range starts in.

- **One file** (default): chapter markers inside Opus, MP3 (ID3 CHAP), M4B and
  FLAC. WAV has no chapter support.
- **Split by chapter** (checkbox next to Format, CLI `--split-chapters`): a folder
  named after Output with `01 - Title.opus`, `02 - …`, each tagged with title and
  track number.
  Re-exporting into the same folder replaces the numbered chapter files of that
  format (the GUI asks first); other files in the folder are not touched.

CLI: `document2voice extract --chapters book.epub` lists what was found.

## Resuming an interrupted export

Full exports save progress after every paragraph in two hidden files next to the
output: `.NAME.document2voice-partial.wav` and `.NAME.document2voice-partial.json`. If the
export is cancelled, crashes or the machine dies, start it again with the **same
document, page range, voice, speed, style and paragraph pause** (and the same
output path) and it continues after the last finished paragraph. The finished
file is identical to an uninterrupted run.

- Format, bitrate, pitch and loudness are applied after synthesis, so changing
  them still resumes.
- Changing anything that alters the speech starts fresh (the existing partial is replaced).
- Both hidden files are deleted when the export succeeds.
- **GUI:** pressing Start when an unfinished export exists for that Output asks
  **Resume** (continue), **Start over** (discard the progress) or Cancel.
- **CLI:** resumes automatically; delete the two hidden files to start over.

## Paragraphs that cannot be spoken

If a paragraph fails to synthesize it is retried once; if it fails again it is
replaced by 0.5 s of silence and the export carries on. When the export finishes,
`NAME.skipped.txt` next to the output lists each skipped paragraph (number,
chapter, error and text), and the Log / CLI shows a warning.

If failures look systematic — more than 3 skipped *and* over 10% of the paragraphs
so far — the export stops with the error instead (a broken espeak-ng or model
would otherwise produce a silent book). Progress is kept, so it resumes once fixed.

## Parallel synthesis (optional tuning)

Full exports run several Kokoro engines at once, each on its own paragraph with
one ONNX Runtime thread, and write the results in order (output is byte-identical
to a single engine). Default: half the CPU threads, max 4, capped so engines use
at most ~60% of free RAM (~600 MB each). Voice previews use one engine.

- `DOCUMENT2VOICE_SYNTH_WORKERS=1` — one engine using all cores
- `DOCUMENT2VOICE_SYNTH_WORKERS=N` — force N engines
- `DOCUMENT2VOICE_ORT_THREADS=N` — ORT threads for single-engine runs

## GUI Output path

If **Output** is left empty, Start writes `output.{ext}` in the launch folder
(or next to the `.AppImage`), picking `output-1.{ext}`, `output-2.{ext}`, … if
that name is taken. A path you typed yourself is asked about before an existing
file is overwritten.

## Pinned build inputs

`build-appimage.sh` pins every download to a version + SHA-256 (see the
"Pinned third-party downloads" block at the top). Conda packages are pinned by
the lockfiles in `locks/` (exact conda-forge URLs with `#sha256:`), which
micromamba verifies:

- `locks/runtime-linux-64.lock` — Poppler + the libraries shipped in the AppImage
- `locks/espeak-build-linux-64.lock` — autotools/compilers used to build espeak-ng (not shipped)

To bump Poppler: create a scratch env with the new version, then regenerate the
lock from its `conda-meta/*.json` (`url` + `sha256` per package):

```bash
micromamba create -y -p /tmp/pop -c conda-forge poppler=<ver>
python3 - /tmp/pop <<'PY' > packaging/appimage/locks/runtime-linux-64.lock
import glob, json, sys
print("# platform: linux-64\n@EXPLICIT")
for f in sorted(glob.glob(sys.argv[1] + "/conda-meta/*.json")):
    j = json.load(open(f)); print(f"{j['url']}#sha256:{j['sha256']}")
PY
```

and update `POPPLER_VER` in `build-appimage.sh`.

## Licenses

`collect-licenses.py` runs during the build and writes every bundled component's
license text plus `THIRD-PARTY-NOTICES.md` to `usr/share/licenses/document2voice/`.
The build fails if any required license source is missing.
