# Part-of-speech tagger data

`weights.json`, `classes.txt`, `tags.json` are the averaged-perceptron POS tagger
model from [misaki-rs](https://github.com/MicheleYin/misaki-rs) 0.3.0
([crates.io](https://crates.io/crates/misaki-rs)). The files are byte-identical to
misaki-rs `src/resources/tagger/`. misaki-rs is MIT licensed per its Cargo metadata
(also listed in the AppImage's `licenses/document2voice/rust/misaki-rs-0.3.0/`).

Used before espeak-ng to tell heteronym senses apart — past-tense "read" (/ɹɛd/) vs
present, and the verb/noun pairs espeak was measured to get wrong (see
`crates/core/src/heteronyms.rs`, test `heteronym_audit_espeak`).
