# DRM test fixtures

Only public-domain or synthetic files; do not add DRM-protected books.

| File | Purpose |
|------|---------|
| `adobe-drm-stub.epub` | Synthetic EPUB with `META-INF/encryption.xml`, so the Adobe DRM detector fires |
| `synthetic-kfx-stub.kfx` | Stub `.kfx` file for the KFX refusal path |

These fixtures exercise detection and the error message only.
