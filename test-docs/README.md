# Test documents

Fixtures for manual smoke tests of Document2Voice (Kokoro).

Only public-domain or synthetic files; do not add DRM-protected books.

## Inventory

### TXT — Project Gutenberg
| File | Source |
|------|--------|
| `txt/alice-in-wonderland.txt` | Gutenberg #11 |
| `txt/frankenstein.txt` | Gutenberg #84 |

### EPUB — Project Gutenberg
| File | Source |
|------|--------|
| `epub/alice-in-wonderland.epub` | Gutenberg #11 |

### MOBI / AZW3 — Project Gutenberg Kindle builds
| File | Source |
|------|--------|
| `mobi/alice-in-wonderland.mobi` | Gutenberg #11 kindle.images |
| `mobi/alice-in-wonderland.azw3` | Gutenberg #11 KF8 (Mobipocket v8) |
| `mobi/frankenstein.mobi` | Gutenberg #84 kindle.images |

### PDF
| File | Source / notes |
|------|----------------|
| `pdf/us-constitution.pdf` | U.S. government (govinfo.gov) public document |
| `pdf/alice-multipage-pd.pdf` | Gutenberg Alice text rendered with Ghostscript |
| `pdf/three-pages-pd.pdf` | 3-page fixture for page range / skip tests |
| `pdf/bookmarked-chapters.pdf` | 4-page synthetic PDF with outline bookmarks (Chapter One/Two) for PDF chapter detection |
| `pdf/scanned-image-only.pdf` | Synthetic image-only PDF (no text layer) for the "needs OCR" error |
| `pdf/speech-sanitize.pdf` | Listening fixture for the speech sanitizer (pass/fail criteria on the page); source `pdf/speech-sanitize.html` |

### DOCX
| File | Source |
|------|--------|
| `docx/alice-chapters.docx` | Synthetic: Alice chapters I–II excerpt with headings, a tab and a table |

### ODT
| File | Source |
|------|--------|
| `odt/alice-chapters.odt` | Synthetic: the same excerpt as ODT |

### DRM fixtures (see `drm/README.md`)
| File | Notes |
|------|--------|
| `drm/adobe-drm-stub.epub` | Synthetic EPUB with `encryption.xml` |
| `drm/synthetic-kfx-stub.kfx` | Synthetic `.kfx` for the KFX refusal path |

## Smoke commands

```bash
export DOCUMENT2VOICE_MODEL_DIR="$(pwd)/models/kokoro"
CLI=target/release/document2voice
DOC=test-docs

$CLI extract "$DOC/txt/alice-in-wonderland.txt" | head
$CLI extract "$DOC/epub/alice-in-wonderland.epub" | head
$CLI extract "$DOC/mobi/alice-in-wonderland.mobi" | head
$CLI extract "$DOC/mobi/alice-in-wonderland.azw3" | head
$CLI extract "$DOC/pdf/three-pages-pd.pdf" --pages 1-2
$CLI extract "$DOC/pdf/bookmarked-chapters.pdf" --chapters
$CLI extract "$DOC/docx/alice-chapters.docx" --chapters
$CLI extract "$DOC/odt/alice-chapters.odt" --chapters

# Expect the scanned / image-only message (no OCR):
$CLI extract "$DOC/pdf/scanned-image-only.pdf"

# Expect a DRM refusal (exit code 2):
$CLI extract "$DOC/drm/adobe-drm-stub.epub"
$CLI extract "$DOC/drm/synthetic-kfx-stub.kfx"
```
