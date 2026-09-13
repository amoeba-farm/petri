# Third-party notices

The September 12 source candidate also embeds a signer-free Node.js SDK runtime.
It also compiles the compressed-evidence verifier from the SDK's immutable native
package export. Its source inventory, lockfile and executable hashes are recorded
inside that runtime. Compilation alone is not proof-fixture qualification; tests
and execution qualification remain deferred. The native helper has no signing or
network interface.
Its exact Node version, SDK Git commit, platform, and per-file hashes are recorded
in the embedded runtime manifest. Node's complete license (`node-LICENSE.txt`),
the SDK's root legal files, and dependency legal files are retained in that
runtime and extracted beneath Petri's private `sdk-runtime` directory. The Rust
inventory below does not purport to cover that additional JavaScript dependency
graph; packaging builds it from the pinned SDK's `package-lock.json`.

Petri includes third-party Rust components. This inventory is generated from the exact locked release dependency graph. Each component remains governed by its listed license; no third-party license is replaced by the Petri Apache-2.0 license.

The complete, lockfile-bound legal corpus shipped with Petri is in [THIRD_PARTY_LICENSES.md](THIRD_PARTY_LICENSES.md). It reproduces every discovered legal file with UTF-8 line endings normalized to LF and labels the pinned SPDX text used when an upstream crate archive omits a legal file or conflicts with its declared license expression. Patched vendored sources retain their upstream license and are documented in vendor/PATCHES.md.

| Component | Version | License expression | Source |
| --- | --- | --- | --- |
| adler2 | 2.0.1 | 0BSD OR MIT OR Apache-2.0 | https://github.com/oyvindln/adler2 |
| aead | 0.5.2 | MIT OR Apache-2.0 | https://github.com/RustCrypto/traits |
| aes | 0.8.4 | MIT OR Apache-2.0 | https://github.com/RustCrypto/block-ciphers |
| aes-gcm-siv | 0.11.1 | MIT OR Apache-2.0 | https://github.com/RustCrypto/AEADs |
| agave-feature-set | 2.3.13 | Apache-2.0 | https://github.com/anza-xyz/agave |
| ahash | 0.8.12 | MIT OR Apache-2.0 | https://github.com/tkaitchuck/ahash |
| aligned-sized | 1.1.0 | Apache-2.0 | https://github.com/Lightprotocol/light-protocol |
| alloc-no-stdlib | 2.0.4 | BSD-3-Clause | https://github.com/dropbox/rust-alloc-no-stdlib |
| alloc-stdlib | 0.2.2 | BSD-3-Clause | https://github.com/dropbox/rust-alloc-no-stdlib |
| allocator-api2 | 0.2.21 | MIT OR Apache-2.0 | https://github.com/zakarumych/allocator-api2 |
| android_system_properties | 0.1.5 | MIT/Apache-2.0 | https://github.com/nical/android_system_properties |
| anstream | 0.6.21 | MIT OR Apache-2.0 | https://github.com/rust-cli/anstyle.git |
| anstyle | 1.0.13 | MIT OR Apache-2.0 | https://github.com/rust-cli/anstyle.git |
| anstyle-parse | 0.2.7 | MIT OR Apache-2.0 | https://github.com/rust-cli/anstyle.git |
| anstyle-query | 1.1.5 | MIT OR Apache-2.0 | https://github.com/rust-cli/anstyle.git |
| anstyle-wincon | 3.0.11 | MIT OR Apache-2.0 | https://github.com/rust-cli/anstyle.git |
| anyhow | 1.0.102 | MIT OR Apache-2.0 | https://github.com/dtolnay/anyhow |
| ark-bn254 | 0.4.0 | MIT/Apache-2.0 | https://github.com/arkworks-rs/curves |
| ark-bn254 | 0.5.0 | MIT OR Apache-2.0 | https://github.com/arkworks-rs/algebra |
| ark-ec | 0.4.2 | MIT/Apache-2.0 | https://github.com/arkworks-rs/algebra |
| ark-ec | 0.5.0 | MIT OR Apache-2.0 | https://github.com/arkworks-rs/algebra |
| ark-ff | 0.4.2 | MIT/Apache-2.0 | https://github.com/arkworks-rs/algebra |
| ark-ff | 0.5.0 | MIT OR Apache-2.0 | https://github.com/arkworks-rs/algebra |
| ark-ff-asm | 0.4.2 | MIT/Apache-2.0 | https://github.com/arkworks-rs/algebra |
| ark-ff-asm | 0.5.0 | MIT OR Apache-2.0 | https://github.com/arkworks-rs/algebra |
| ark-ff-macros | 0.4.2 | MIT/Apache-2.0 | https://github.com/arkworks-rs/algebra |
| ark-ff-macros | 0.5.0 | MIT OR Apache-2.0 | https://github.com/arkworks-rs/algebra |
| ark-poly | 0.4.2 | MIT/Apache-2.0 | https://github.com/arkworks-rs/algebra |
| ark-poly | 0.5.0 | MIT OR Apache-2.0 | https://github.com/arkworks-rs/algebra |
| ark-serialize | 0.4.2 | MIT/Apache-2.0 | https://github.com/arkworks-rs/algebra |
| ark-serialize | 0.5.0 | MIT OR Apache-2.0 | https://github.com/arkworks-rs/algebra |
| ark-serialize-derive | 0.4.2 | MIT/Apache-2.0 | https://github.com/arkworks-rs/algebra |
| ark-serialize-derive | 0.5.0 | MIT OR Apache-2.0 | https://github.com/arkworks-rs/algebra |
| ark-std | 0.4.0 | MIT/Apache-2.0 | https://github.com/arkworks-rs/std |
| ark-std | 0.5.0 | MIT/Apache-2.0 | https://github.com/arkworks-rs/std |
| arraydeque | 0.5.1 | MIT/Apache-2.0 | https://github.com/andylokandy/arraydeque |
| arrayref | 0.3.9 | BSD-2-Clause | https://github.com/droundy/arrayref |
| arrayvec | 0.7.6 | MIT OR Apache-2.0 | https://github.com/bluss/arrayvec |
| async-compression | 0.4.41 | MIT OR Apache-2.0 | https://github.com/Nullus157/async-compression |
| async-trait | 0.1.89 | MIT OR Apache-2.0 | https://github.com/dtolnay/async-trait |
| atomic-waker | 1.1.2 | Apache-2.0 OR MIT | https://github.com/smol-rs/atomic-waker |
| autocfg | 1.5.0 | Apache-2.0 OR MIT | https://github.com/cuviper/autocfg |
| aws-lc-rs | 1.17.1 | ISC AND (Apache-2.0 OR ISC) | https://github.com/aws/aws-lc-rs |
| aws-lc-sys | 0.42.0 | ISC AND (Apache-2.0 OR ISC) AND Apache-2.0 AND MIT AND BSD-3-Clause AND (Apache-2.0 OR ISC OR MIT) AND (Apache-2.0 OR ISC OR MIT-0) | https://github.com/aws/aws-lc-rs |
| base64 | 0.12.3 | MIT/Apache-2.0 | https://github.com/marshallpierce/rust-base64 |
| base64 | 0.13.1 | MIT/Apache-2.0 | https://github.com/marshallpierce/rust-base64 |
| base64 | 0.22.1 | MIT OR Apache-2.0 | https://github.com/marshallpierce/rust-base64 |
| base64ct | 1.8.3 | Apache-2.0 OR MIT | https://github.com/RustCrypto/formats |
| bincode | 1.3.3 | MIT | https://github.com/servo/bincode |
| bitflags | 2.11.0 | MIT OR Apache-2.0 | https://github.com/bitflags/bitflags |
| bitvec | 1.0.1 | MIT | https://github.com/bitvecto-rs/bitvec |
| blake3 | 1.8.3 | CC0-1.0 OR Apache-2.0 OR Apache-2.0 WITH LLVM-exception | https://github.com/BLAKE3-team/BLAKE3 |
| block-buffer | 0.10.4 | MIT OR Apache-2.0 | https://github.com/RustCrypto/utils |
| block-buffer | 0.9.0 | MIT OR Apache-2.0 | https://github.com/RustCrypto/utils |
| borsh | 0.10.4 | MIT OR Apache-2.0 | https://github.com/near/borsh-rs |
| borsh | 1.6.0 | MIT OR Apache-2.0 | https://github.com/near/borsh-rs |
| borsh-derive | 0.10.4 | Apache-2.0 | https://github.com/nearprotocol/borsh |
| borsh-derive | 1.6.0 | Apache-2.0 | https://github.com/near/borsh-rs |
| borsh-derive-internal | 0.10.4 | Apache-2.0 | https://github.com/nearprotocol/borsh |
| borsh-schema-derive-internal | 0.10.4 | Apache-2.0 | https://github.com/nearprotocol/borsh |
| brotli | 8.0.2 | BSD-3-Clause AND MIT | https://github.com/dropbox/rust-brotli |
| brotli-decompressor | 5.0.0 | BSD-3-Clause/MIT | https://github.com/dropbox/rust-brotli-decompressor |
| bs58 | 0.5.1 | MIT/Apache-2.0 | https://github.com/Nullus157/bs58-rs |
| bumpalo | 3.20.2 | MIT OR Apache-2.0 | https://github.com/fitzgen/bumpalo |
| bv | 0.11.1 | MIT/Apache-2.0 | https://github.com/tov/bv-rs |
| bytemuck | 1.25.0 | Zlib OR Apache-2.0 OR MIT | https://github.com/Lokathor/bytemuck |
| bytemuck_derive | 1.10.2 | Zlib OR Apache-2.0 OR MIT | https://github.com/Lokathor/bytemuck |
| byteorder | 1.5.0 | Unlicense OR MIT | https://github.com/BurntSushi/byteorder |
| bytes | 1.11.1 | MIT | https://github.com/tokio-rs/bytes |
| cassowary | 0.3.0 | MIT / Apache-2.0 | https://github.com/dylanede/cassowary-rs |
| castaway | 0.2.4 | MIT | https://github.com/sagebind/castaway |
| cc | 1.2.56 | MIT OR Apache-2.0 | https://github.com/rust-lang/cc-rs |
| cfg_aliases | 0.2.1 | MIT | https://github.com/katharostech/cfg_aliases |
| cfg-if | 1.0.4 | MIT OR Apache-2.0 | https://github.com/rust-lang/cfg-if |
| chrono | 0.4.44 | MIT OR Apache-2.0 | https://github.com/chronotope/chrono |
| cipher | 0.4.4 | MIT OR Apache-2.0 | https://github.com/RustCrypto/traits |
| clap | 4.5.60 | MIT OR Apache-2.0 | https://github.com/clap-rs/clap |
| clap_builder | 4.5.60 | MIT OR Apache-2.0 | https://github.com/clap-rs/clap |
| clap_derive | 4.5.55 | MIT OR Apache-2.0 | https://github.com/clap-rs/clap |
| clap_lex | 1.0.0 | MIT OR Apache-2.0 | https://github.com/clap-rs/clap |
| cmake | 0.1.58 | MIT OR Apache-2.0 | https://github.com/rust-lang/cmake-rs |
| colorchoice | 1.0.4 | MIT OR Apache-2.0 | https://github.com/rust-cli/anstyle.git |
| combine | 4.6.7 | MIT | https://github.com/Marwes/combine |
| compact_str | 0.8.2 | MIT | https://github.com/ParkMyCar/compact_str |
| compression-codecs | 0.4.37 | MIT OR Apache-2.0 | https://github.com/Nullus157/async-compression |
| compression-core | 0.4.31 | MIT OR Apache-2.0 | https://github.com/Nullus157/async-compression |
| console | 0.15.11 | MIT | https://github.com/console-rs/console |
| console_error_panic_hook | 0.1.7 | Apache-2.0/MIT | https://github.com/rustwasm/console_error_panic_hook |
| console_log | 0.2.2 | MIT/Apache-2.0 | https://github.com/iamcodemaker/console_log |
| const-oid | 0.9.6 | Apache-2.0 OR MIT | https://github.com/RustCrypto/formats/tree/master/const-oid |
| constant_time_eq | 0.4.2 | CC0-1.0 OR MIT-0 OR Apache-2.0 | https://github.com/cesarb/constant_time_eq |
| core-foundation | 0.10.1 | MIT OR Apache-2.0 | https://github.com/servo/core-foundation-rs |
| core-foundation | 0.9.4 | MIT OR Apache-2.0 | https://github.com/servo/core-foundation-rs |
| core-foundation-sys | 0.8.7 | MIT OR Apache-2.0 | https://github.com/servo/core-foundation-rs |
| cpufeatures | 0.2.17 | MIT OR Apache-2.0 | https://github.com/RustCrypto/utils |
| crc32fast | 1.5.0 | MIT OR Apache-2.0 | https://github.com/srijs/rust-crc32fast |
| crossbeam-deque | 0.8.6 | MIT OR Apache-2.0 | https://github.com/crossbeam-rs/crossbeam |
| crossbeam-epoch | 0.9.20 | MIT OR Apache-2.0 | https://github.com/crossbeam-rs/crossbeam |
| crossbeam-utils | 0.8.21 | MIT OR Apache-2.0 | https://github.com/crossbeam-rs/crossbeam |
| crossterm | 0.28.1 | MIT | https://github.com/crossterm-rs/crossterm |
| crossterm_winapi | 0.9.1 | MIT | https://github.com/crossterm-rs/crossterm-winapi |
| crunchy | 0.2.4 | MIT | https://github.com/eira-fransham/crunchy |
| crypto-common | 0.1.7 | MIT OR Apache-2.0 | https://github.com/RustCrypto/traits |
| ctr | 0.9.2 | MIT OR Apache-2.0 | https://github.com/RustCrypto/block-modes |
| curve25519-dalek | 4.1.3 | BSD-3-Clause | https://github.com/dalek-cryptography/curve25519-dalek/tree/main/curve25519-dalek |
| curve25519-dalek-derive | 0.1.1 | MIT/Apache-2.0 | https://github.com/dalek-cryptography/curve25519-dalek |
| darling | 0.21.3 | MIT | https://github.com/TedDriggs/darling |
| darling | 0.23.0 | MIT | https://github.com/TedDriggs/darling |
| darling_core | 0.21.3 | MIT | https://github.com/TedDriggs/darling |
| darling_core | 0.23.0 | MIT | https://github.com/TedDriggs/darling |
| darling_macro | 0.21.3 | MIT | https://github.com/TedDriggs/darling |
| darling_macro | 0.23.0 | MIT | https://github.com/TedDriggs/darling |
| der | 0.7.10 | Apache-2.0 OR MIT | https://github.com/RustCrypto/formats/tree/master/der |
| derivation-path | 0.2.0 | MIT OR Apache-2.0 | https://github.com/jpopesculian/derivation-path |
| derivative | 2.2.0 | MIT/Apache-2.0 | https://github.com/mcarton/rust-derivative |
| dialoguer | 0.10.4 | MIT | https://github.com/mitsuhiko/dialoguer |
| digest | 0.10.7 | MIT OR Apache-2.0 | https://github.com/RustCrypto/traits |
| digest | 0.9.0 | MIT OR Apache-2.0 | https://github.com/RustCrypto/traits |
| displaydoc | 0.2.5 | MIT OR Apache-2.0 | https://github.com/yaahc/displaydoc |
| dunce | 1.0.5 | CC0-1.0 OR MIT-0 OR Apache-2.0 | https://gitlab.com/kornelski/dunce |
| ed25519 | 2.2.3 | Apache-2.0 OR MIT | https://github.com/RustCrypto/signatures/tree/master/ed25519 |
| ed25519-dalek | 2.2.0 | BSD-3-Clause | https://github.com/dalek-cryptography/curve25519-dalek/tree/main/ed25519-dalek |
| educe | 0.6.0 | MIT | https://github.com/magiclen/educe |
| either | 1.15.0 | MIT OR Apache-2.0 | https://github.com/rayon-rs/either |
| encode_unicode | 1.0.0 | Apache-2.0 OR MIT | https://github.com/tormol/encode_unicode |
| encoding_rs | 0.8.35 | (Apache-2.0 OR MIT) AND BSD-3-Clause | https://github.com/hsivonen/encoding_rs |
| enum-ordinalize | 4.3.2 | MIT | https://github.com/magiclen/enum-ordinalize |
| enum-ordinalize-derive | 4.3.2 | MIT | https://github.com/magiclen/enum-ordinalize |
| equivalent | 1.0.2 | Apache-2.0 OR MIT | https://github.com/indexmap-rs/equivalent |
| errno | 0.3.14 | MIT OR Apache-2.0 | https://github.com/lambda-fairy/rust-errno |
| fastrand | 2.3.0 | Apache-2.0 OR MIT | https://github.com/smol-rs/fastrand |
| feature-probe | 0.1.1 | MIT/Apache-2.0 | https://github.com/tov/feature-probe-rs |
| fiat-crypto | 0.2.9 | MIT OR Apache-2.0 OR BSD-1-Clause | https://github.com/mit-plv/fiat-crypto |
| find-msvc-tools | 0.1.9 | MIT OR Apache-2.0 | https://github.com/rust-lang/cc-rs |
| five8 | 0.2.1 | MIT | https://github.com/kevinheavey/five8 |
| five8_const | 0.1.4 | MIT | https://github.com/kevinheavey/five8 |
| five8_core | 0.1.2 | MIT | https://github.com/kevinheavey/five8 |
| flate2 | 1.1.9 | MIT OR Apache-2.0 | https://github.com/rust-lang/flate2-rs |
| fnv | 1.0.7 | Apache-2.0 / MIT | https://github.com/servo/rust-fnv |
| foldhash | 0.1.5 | Zlib | https://github.com/orlp/foldhash |
| foldhash | 0.2.0 | Zlib | https://github.com/orlp/foldhash |
| foreign-types | 0.3.2 | MIT/Apache-2.0 | https://github.com/sfackler/foreign-types |
| foreign-types-shared | 0.1.1 | MIT/Apache-2.0 | https://github.com/sfackler/foreign-types |
| form_urlencoded | 1.2.2 | MIT OR Apache-2.0 | https://github.com/servo/rust-url |
| fs_extra | 1.3.0 | MIT | https://github.com/webdesus/fs_extra |
| funty | 2.0.0 | MIT | https://github.com/myrrlyn/funty |
| futures | 0.3.32 | MIT OR Apache-2.0 | https://github.com/rust-lang/futures-rs |
| futures-channel | 0.3.32 | MIT OR Apache-2.0 | https://github.com/rust-lang/futures-rs |
| futures-core | 0.3.32 | MIT OR Apache-2.0 | https://github.com/rust-lang/futures-rs |
| futures-executor | 0.3.32 | MIT OR Apache-2.0 | https://github.com/rust-lang/futures-rs |
| futures-io | 0.3.32 | MIT OR Apache-2.0 | https://github.com/rust-lang/futures-rs |
| futures-macro | 0.3.32 | MIT OR Apache-2.0 | https://github.com/rust-lang/futures-rs |
| futures-sink | 0.3.32 | MIT OR Apache-2.0 | https://github.com/rust-lang/futures-rs |
| futures-task | 0.3.32 | MIT OR Apache-2.0 | https://github.com/rust-lang/futures-rs |
| futures-util | 0.3.32 | MIT OR Apache-2.0 | https://github.com/rust-lang/futures-rs |
| generic-array | 0.14.7 | MIT | https://github.com/fizyk20/generic-array.git |
| getrandom | 0.1.16 | MIT OR Apache-2.0 | https://github.com/rust-random/getrandom |
| getrandom | 0.2.17 | MIT OR Apache-2.0 | https://github.com/rust-random/getrandom |
| getrandom | 0.3.4 | MIT OR Apache-2.0 | https://github.com/rust-random/getrandom |
| getrandom | 0.4.2 | MIT OR Apache-2.0 | https://github.com/rust-random/getrandom |
| groth16-solana | 0.2.0 | MIT | https://github.com/Lightprotocol/groth16-solana |
| h2 | 0.4.13 | MIT | https://github.com/hyperium/h2 |
| hashbrown | 0.13.2 | MIT OR Apache-2.0 | https://github.com/rust-lang/hashbrown |
| hashbrown | 0.15.2 | MIT OR Apache-2.0 | https://github.com/rust-lang/hashbrown |
| hashbrown | 0.16.1 | MIT OR Apache-2.0 | https://github.com/rust-lang/hashbrown |
| hashlink | 0.11.0 | MIT OR Apache-2.0 | https://github.com/kyren/hashlink |
| heck | 0.5.0 | MIT OR Apache-2.0 | https://github.com/withoutboats/heck |
| hidapi | 2.6.6 | MIT | https://github.com/ruabmbua/hidapi-rs |
| hmac | 0.12.1 | MIT OR Apache-2.0 | https://github.com/RustCrypto/MACs |
| http | 1.4.0 | MIT OR Apache-2.0 | https://github.com/hyperium/http |
| http-body | 1.0.1 | MIT | https://github.com/hyperium/http-body |
| http-body-util | 0.1.3 | MIT | https://github.com/hyperium/http-body |
| httparse | 1.10.1 | MIT OR Apache-2.0 | https://github.com/seanmonstar/httparse |
| hyper | 1.8.1 | MIT | https://github.com/hyperium/hyper |
| hyper-rustls | 0.27.7 | Apache-2.0 OR ISC OR MIT | https://github.com/rustls/hyper-rustls |
| hyper-tls | 0.6.0 | MIT/Apache-2.0 | https://github.com/hyperium/hyper-tls |
| hyper-util | 0.1.20 | MIT | https://github.com/hyperium/hyper-util |
| iana-time-zone | 0.1.65 | MIT OR Apache-2.0 | https://github.com/strawlab/iana-time-zone |
| iana-time-zone-haiku | 0.1.2 | MIT OR Apache-2.0 | https://github.com/strawlab/iana-time-zone |
| icu_collections | 2.1.1 | Unicode-3.0 | https://github.com/unicode-org/icu4x |
| icu_locale_core | 2.1.1 | Unicode-3.0 | https://github.com/unicode-org/icu4x |
| icu_normalizer | 2.1.1 | Unicode-3.0 | https://github.com/unicode-org/icu4x |
| icu_normalizer_data | 2.1.1 | Unicode-3.0 | https://github.com/unicode-org/icu4x |
| icu_properties | 2.1.2 | Unicode-3.0 | https://github.com/unicode-org/icu4x |
| icu_properties_data | 2.1.2 | Unicode-3.0 | https://github.com/unicode-org/icu4x |
| icu_provider | 2.1.1 | Unicode-3.0 | https://github.com/unicode-org/icu4x |
| id-arena | 2.3.0 | MIT/Apache-2.0 | https://github.com/fitzgen/id-arena |
| ident_case | 1.0.1 | MIT/Apache-2.0 | https://github.com/TedDriggs/ident_case |
| idna | 1.1.0 | MIT OR Apache-2.0 | https://github.com/servo/rust-url/ |
| idna_adapter | 1.2.1 | Apache-2.0 OR MIT | https://github.com/hsivonen/idna_adapter |
| indexmap | 2.13.0 | Apache-2.0 OR MIT | https://github.com/indexmap-rs/indexmap |
| indicatif | 0.17.11 | MIT | https://github.com/console-rs/indicatif |
| indoc | 2.0.7 | MIT OR Apache-2.0 | https://github.com/dtolnay/indoc |
| inout | 0.1.4 | MIT OR Apache-2.0 | https://github.com/RustCrypto/utils |
| instability | 0.3.12 | MIT | https://github.com/ratatui/instability |
| ipnet | 2.12.0 | MIT OR Apache-2.0 | https://github.com/krisprice/ipnet |
| iri-string | 0.7.10 | MIT OR Apache-2.0 | https://github.com/lo48576/iri-string |
| is_terminal_polyfill | 1.70.2 | MIT OR Apache-2.0 | https://github.com/polyfill-rs/is_terminal_polyfill |
| itertools | 0.10.5 | MIT/Apache-2.0 | https://github.com/rust-itertools/itertools |
| itertools | 0.12.1 | MIT OR Apache-2.0 | https://github.com/rust-itertools/itertools |
| itertools | 0.13.0 | MIT OR Apache-2.0 | https://github.com/rust-itertools/itertools |
| itoa | 1.0.17 | MIT OR Apache-2.0 | https://github.com/dtolnay/itoa |
| jni | 0.22.4 | MIT OR Apache-2.0 | https://github.com/jni-rs/jni-rs |
| jni-macros | 0.22.4 | MIT OR Apache-2.0 | https://github.com/jni-rs/jni-rs |
| jni-sys | 0.4.1 | MIT OR Apache-2.0 | https://github.com/jni-rs/jni-sys |
| jni-sys-macros | 0.4.1 | MIT OR Apache-2.0 | https://github.com/jni-rs/jni-sys |
| jobserver | 0.1.34 | MIT OR Apache-2.0 | https://github.com/rust-lang/jobserver-rs |
| js-sys | 0.3.91 | MIT OR Apache-2.0 | https://github.com/wasm-bindgen/wasm-bindgen/tree/master/crates/js-sys |
| jsonrpc-core | 18.0.0 | MIT | https://github.com/paritytech/jsonrpc |
| keccak | 0.1.6 | Apache-2.0 OR MIT | https://github.com/RustCrypto/sponges/tree/master/keccak |
| lazy_static | 1.5.0 | MIT OR Apache-2.0 | https://github.com/rust-lang-nursery/lazy-static.rs |
| leb128fmt | 0.1.0 | MIT OR Apache-2.0 | https://github.com/bluk/leb128fmt |
| libc | 0.2.183 | MIT OR Apache-2.0 | https://github.com/rust-lang/libc |
| libsecp256k1 | 0.6.0 | Apache-2.0 | https://github.com/paritytech/libsecp256k1 |
| libsecp256k1-core | 0.2.2 | Apache-2.0 | https://github.com/paritytech/libsecp256k1 |
| libsecp256k1-gen-ecmult | 0.2.1 | Apache-2.0 | https://github.com/paritytech/libsecp256k1 |
| libsecp256k1-gen-genmult | 0.2.1 | Apache-2.0 | https://github.com/paritytech/libsecp256k1 |
| light-account | 0.23.0 | Apache-2.0 | https://github.com/Lightprotocol/light-protocol |
| light-account-checks | 0.8.0 | Apache-2.0 | https://github.com/Lightprotocol/light-protocol |
| light-array-map | 0.2.0 | Apache-2.0 | https://github.com/Lightprotocol/light-protocol |
| light-batched-merkle-tree | 0.11.0 | Apache-2.0 | https://github.com/Lightprotocol/light-protocol |
| light-bloom-filter | 0.6.0 | Apache-2.0 | https://github.com/Lightprotocol/light-protocol |
| light-bounded-vec | 2.0.1 | Apache-2.0 | https://github.com/Lightprotocol/light-protocol |
| light-client | 0.23.0 | Apache-2.0 | https://github.com/lightprotocol/light-protocol |
| light-compressed-account | 0.11.0 | Apache-2.0 | https://github.com/Lightprotocol/light-protocol |
| light-compressed-token-sdk | 0.23.0 | Apache-2.0 | https://github.com/Lightprotocol/light-protocol |
| light-compressible | 0.6.0 | MIT | https://crates.io/crates/light-compressible/0.6.0 |
| light-concurrent-merkle-tree | 5.0.0 | Apache-2.0 | https://github.com/Lightprotocol/light-protocol |
| light-event | 0.23.1 | Apache-2.0 | https://github.com/Lightprotocol/light-protocol |
| light-hasher | 5.0.0 | Apache-2.0 | https://github.com/Lightprotocol/light-protocol |
| light-indexed-array | 0.3.0 | Apache-2.0 | https://github.com/Lightprotocol/light-protocol |
| light-indexed-merkle-tree | 5.0.0 | Apache-2.0 | https://github.com/Lightprotocol/light-protocol |
| light-macros | 2.2.0 | Apache-2.0 | https://github.com/Lightprotocol/light-protocol |
| light-merkle-tree-metadata | 0.11.0 | Apache-2.0 | https://github.com/Lightprotocol/light-protocol |
| light-merkle-tree-reference | 4.0.0 | Apache-2.0 | https://github.com/Lightprotocol/light-protocol |
| light-poseidon | 0.3.0 | Apache-2.0 | https://github.com/Lightprotocol/light-poseidon |
| light-profiler-macro | 0.1.1 | Apache-2.0 | https://github.com/Lightprotocol/light-program-profiler |
| light-program-profiler | 0.1.1 | Apache-2.0 | https://github.com/Lightprotocol/light-program-profiler |
| light-prover-client | 8.0.0 | Apache-2.0 | https://github.com/Lightprotocol/light-protocol |
| light-sdk | 0.23.0 | Apache-2.0 | https://github.com/Lightprotocol/light-protocol |
| light-sdk-macros | 0.23.0 | Apache-2.0 | https://github.com/Lightprotocol/light-protocol |
| light-sdk-types | 0.23.0 | Apache-2.0 | https://github.com/lightprotocol/light-protocol |
| light-sparse-merkle-tree | 0.3.0 | Apache-2.0 | https://github.com/Lightprotocol/light-protocol |
| light-token | 0.23.0 | Apache-2.0 | https://github.com/Lightprotocol/light-protocol |
| light-token-interface | 0.5.0 | MIT | https://crates.io/crates/light-token-interface/0.5.0 |
| light-token-types | 0.23.0 | Apache-2.0 | https://github.com/Lightprotocol/light-protocol |
| light-verifier | 10.0.0 | Apache-2.0 | https://github.com/Lightprotocol/light-protocol |
| light-zero-copy | 0.6.0 | Apache-2.0 | https://github.com/Lightprotocol/light-protocol |
| light-zero-copy-derive | 0.6.0 | Apache-2.0 | https://crates.io/crates/light-zero-copy-derive/0.6.0 |
| linux-raw-sys | 0.12.1 | Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT | https://github.com/sunfishcode/linux-raw-sys |
| linux-raw-sys | 0.4.15 | Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT | https://github.com/sunfishcode/linux-raw-sys |
| litemap | 0.8.1 | Unicode-3.0 | https://github.com/unicode-org/icu4x |
| lock_api | 0.4.14 | MIT OR Apache-2.0 | https://github.com/Amanieu/parking_lot |
| log | 0.4.29 | MIT OR Apache-2.0 | https://github.com/rust-lang/log |
| lru | 0.12.5 | MIT | https://github.com/jeromefroe/lru-rs.git |
| lru-slab | 0.1.2 | MIT OR Apache-2.0 OR Zlib | https://github.com/Ralith/lru-slab |
| memchr | 2.8.0 | Unlicense OR MIT | https://github.com/BurntSushi/memchr |
| memoffset | 0.9.1 | MIT | https://github.com/Gilnaa/memoffset |
| merlin | 3.0.0 | MIT | https://github.com/zkcrypto/merlin |
| mime | 0.3.17 | MIT OR Apache-2.0 | https://github.com/hyperium/mime |
| miniz_oxide | 0.8.9 | MIT OR Zlib OR Apache-2.0 | https://github.com/Frommi/miniz_oxide/tree/master/miniz_oxide |
| mio | 1.1.1 | MIT | https://github.com/tokio-rs/mio |
| native-tls | 0.2.18 | MIT OR Apache-2.0 | https://github.com/rust-native-tls/rust-native-tls |
| num_enum | 0.7.5 | BSD-3-Clause OR MIT OR Apache-2.0 | https://github.com/illicitonion/num_enum |
| num_enum_derive | 0.7.5 | BSD-3-Clause OR MIT OR Apache-2.0 | https://github.com/illicitonion/num_enum |
| num-bigint | 0.4.8 | MIT OR Apache-2.0 | https://github.com/rust-num/num-bigint |
| num-derive | 0.4.2 | MIT OR Apache-2.0 | https://github.com/rust-num/num-derive |
| num-integer | 0.1.46 | MIT OR Apache-2.0 | https://github.com/rust-num/num-integer |
| num-traits | 0.2.19 | MIT OR Apache-2.0 | https://github.com/rust-num/num-traits |
| number_prefix | 0.4.0 | MIT | https://github.com/ogham/rust-number-prefix |
| once_cell | 1.21.3 | MIT OR Apache-2.0 | https://github.com/matklad/once_cell |
| once_cell_polyfill | 1.70.2 | MIT OR Apache-2.0 | https://github.com/polyfill-rs/once_cell_polyfill |
| opaque-debug | 0.3.1 | MIT OR Apache-2.0 | https://github.com/RustCrypto/utils |
| openssl | 0.10.75 | Apache-2.0 | https://github.com/rust-openssl/rust-openssl |
| openssl-macros | 0.1.1 | MIT/Apache-2.0 | https://crates.io/crates/openssl-macros/0.1.1 |
| openssl-probe | 0.2.1 | MIT OR Apache-2.0 | https://github.com/rustls/openssl-probe |
| openssl-sys | 0.9.111 | MIT | https://github.com/rust-openssl/rust-openssl |
| parking_lot | 0.12.5 | MIT OR Apache-2.0 | https://github.com/Amanieu/parking_lot |
| parking_lot_core | 0.9.12 | MIT OR Apache-2.0 | https://github.com/Amanieu/parking_lot |
| paste | 1.0.15 | MIT OR Apache-2.0 | https://github.com/dtolnay/paste |
| pbkdf2 | 0.11.0 | MIT OR Apache-2.0 | https://github.com/RustCrypto/password-hashes/tree/master/pbkdf2 |
| percent-encoding | 2.3.2 | MIT OR Apache-2.0 | https://github.com/servo/rust-url/ |
| photon-api | 0.56.0 | Apache-2.0 | https://crates.io/crates/photon-api/0.56.0 |
| pin-project-lite | 0.2.17 | Apache-2.0 OR MIT | https://github.com/taiki-e/pin-project-lite |
| pin-utils | 0.1.0 | MIT OR Apache-2.0 | https://github.com/rust-lang-nursery/pin-utils |
| pinocchio | 0.9.2 | Apache-2.0 | https://github.com/anza-xyz/pinocchio |
| pinocchio-pubkey | 0.3.0 | Apache-2.0 | https://github.com/anza-xyz/pinocchio |
| pkcs8 | 0.10.2 | Apache-2.0 OR MIT | https://github.com/RustCrypto/formats/tree/master/pkcs8 |
| pkg-config | 0.3.32 | MIT OR Apache-2.0 | https://github.com/rust-lang/pkg-config-rs |
| polyval | 0.6.2 | Apache-2.0 OR MIT | https://github.com/RustCrypto/universal-hashes |
| portable-atomic | 1.13.1 | Apache-2.0 OR MIT | https://github.com/taiki-e/portable-atomic |
| potential_utf | 0.1.4 | Unicode-3.0 | https://github.com/unicode-org/icu4x |
| ppv-lite86 | 0.2.21 | MIT OR Apache-2.0 | https://github.com/cryptocorrosion/cryptocorrosion |
| prettyplease | 0.2.37 | MIT OR Apache-2.0 | https://github.com/dtolnay/prettyplease |
| proc-macro-crate | 0.1.5 | Apache-2.0/MIT | https://github.com/bkchr/proc-macro-crate |
| proc-macro-crate | 3.5.0 | MIT OR Apache-2.0 | https://github.com/bkchr/proc-macro-crate |
| proc-macro2 | 1.0.106 | MIT OR Apache-2.0 | https://github.com/dtolnay/proc-macro2 |
| progenitor-client | 0.12.0 | MPL-2.0 | https://github.com/oxidecomputer/progenitor.git |
| qstring | 0.7.2 | MIT | https://github.com/algesten/qstring |
| quinn | 0.11.9 | MIT OR Apache-2.0 | https://github.com/quinn-rs/quinn |
| quinn-proto | 0.11.15 | MIT OR Apache-2.0 | https://github.com/quinn-rs/quinn |
| quinn-udp | 0.5.14 | MIT OR Apache-2.0 | https://github.com/quinn-rs/quinn |
| quote | 1.0.45 | MIT OR Apache-2.0 | https://github.com/dtolnay/quote |
| r-efi | 5.3.0 | MIT OR Apache-2.0 OR LGPL-2.1-or-later | https://github.com/r-efi/r-efi |
| r-efi | 6.0.0 | MIT OR Apache-2.0 OR LGPL-2.1-or-later | https://github.com/r-efi/r-efi |
| radium | 0.7.0 | MIT | https://github.com/bitvecto-rs/radium |
| rand | 0.7.3 | MIT OR Apache-2.0 | https://github.com/rust-random/rand |
| rand | 0.8.5 | MIT OR Apache-2.0 | https://github.com/rust-random/rand |
| rand | 0.9.2 | MIT OR Apache-2.0 | https://github.com/rust-random/rand |
| rand_chacha | 0.2.2 | MIT OR Apache-2.0 | https://github.com/rust-random/rand |
| rand_chacha | 0.3.1 | MIT OR Apache-2.0 | https://github.com/rust-random/rand |
| rand_chacha | 0.9.0 | MIT OR Apache-2.0 | https://github.com/rust-random/rand |
| rand_core | 0.5.1 | MIT OR Apache-2.0 | https://github.com/rust-random/rand |
| rand_core | 0.6.4 | MIT OR Apache-2.0 | https://github.com/rust-random/rand |
| rand_core | 0.9.5 | MIT OR Apache-2.0 | https://github.com/rust-random/rand |
| rand_hc | 0.2.0 | MIT/Apache-2.0 | https://github.com/rust-random/rand |
| ratatui | 0.29.0 | MIT | https://github.com/ratatui/ratatui |
| rayon | 1.11.0 | MIT OR Apache-2.0 | https://github.com/rayon-rs/rayon |
| rayon-core | 1.13.0 | MIT OR Apache-2.0 | https://github.com/rayon-rs/rayon |
| redox_syscall | 0.5.18 | MIT | https://gitlab.redox-os.org/redox-os/syscall |
| reqwest | 0.12.28 | MIT OR Apache-2.0 | https://github.com/seanmonstar/reqwest |
| reqwest | 0.13.4 | MIT OR Apache-2.0 | https://github.com/seanmonstar/reqwest |
| reqwest-middleware | 0.4.2 | MIT OR Apache-2.0 | https://github.com/TrueLayer/reqwest-middleware |
| ring | 0.17.14 | Apache-2.0 AND ISC | https://github.com/briansmith/ring |
| rustc_version | 0.4.1 | MIT OR Apache-2.0 | https://github.com/djc/rustc-version-rs |
| rustc-hash | 2.1.1 | Apache-2.0 OR MIT | https://github.com/rust-lang/rustc-hash |
| rustix | 0.38.44 | Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT | https://github.com/bytecodealliance/rustix |
| rustix | 1.1.4 | Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT | https://github.com/bytecodealliance/rustix |
| rustls | 0.23.37 | Apache-2.0 OR ISC OR MIT | https://github.com/rustls/rustls |
| rustls-native-certs | 0.8.4 | Apache-2.0 OR ISC OR MIT | https://github.com/rustls/rustls-native-certs |
| rustls-pki-types | 1.14.0 | MIT OR Apache-2.0 | https://github.com/rustls/pki-types |
| rustls-platform-verifier | 0.7.0 | MIT OR Apache-2.0 | https://github.com/rustls/rustls-platform-verifier |
| rustls-platform-verifier-android | 0.1.1 | MIT OR Apache-2.0 | https://github.com/rustls/rustls-platform-verifier |
| rustls-webpki | 0.103.13 | ISC | https://github.com/rustls/webpki |
| rustversion | 1.0.22 | MIT OR Apache-2.0 | https://github.com/dtolnay/rustversion |
| ryu | 1.0.23 | Apache-2.0 OR BSL-1.0 | https://github.com/dtolnay/ryu |
| same-file | 1.0.6 | Unlicense/MIT | https://github.com/BurntSushi/same-file |
| schannel | 0.1.29 | MIT | https://github.com/steffengy/schannel-rs |
| scopeguard | 1.2.0 | MIT OR Apache-2.0 | https://github.com/bluss/scopeguard |
| security-framework | 3.7.0 | MIT OR Apache-2.0 | https://github.com/kornelski/rust-security-framework |
| security-framework-sys | 2.17.0 | MIT OR Apache-2.0 | https://github.com/kornelski/rust-security-framework |
| semver | 1.0.27 | MIT OR Apache-2.0 | https://github.com/dtolnay/semver |
| serde | 1.0.228 | MIT OR Apache-2.0 | https://github.com/serde-rs/serde |
| serde_bytes | 0.11.19 | MIT OR Apache-2.0 | https://github.com/serde-rs/bytes |
| serde_core | 1.0.228 | MIT OR Apache-2.0 | https://github.com/serde-rs/serde |
| serde_derive | 1.0.228 | MIT OR Apache-2.0 | https://github.com/serde-rs/serde |
| serde_json | 1.0.149 | MIT OR Apache-2.0 | https://github.com/serde-rs/json |
| serde_spanned | 1.1.1 | MIT OR Apache-2.0 | https://github.com/toml-rs/toml |
| serde_urlencoded | 0.7.1 | MIT/Apache-2.0 | https://github.com/nox/serde_urlencoded |
| serde-big-array | 0.5.1 | MIT OR Apache-2.0 | https://github.com/est31/serde-big-array |
| sha2 | 0.10.9 | MIT OR Apache-2.0 | https://github.com/RustCrypto/hashes |
| sha2 | 0.9.9 | MIT OR Apache-2.0 | https://github.com/RustCrypto/hashes |
| sha2-const-stable | 0.1.0 | MIT OR Apache-2.0 | https://github.com/saleemrashid/sha2-const |
| sha3 | 0.10.8 | MIT OR Apache-2.0 | https://github.com/RustCrypto/hashes |
| shell-words | 1.1.1 | MIT/Apache-2.0 | https://github.com/tmiasko/shell-words |
| shlex | 1.3.0 | MIT OR Apache-2.0 | https://github.com/comex/rust-shlex |
| signal-hook | 0.3.18 | Apache-2.0/MIT | https://github.com/vorner/signal-hook |
| signal-hook-mio | 0.2.5 | MIT OR Apache-2.0 | https://github.com/vorner/signal-hook |
| signal-hook-registry | 1.4.8 | MIT OR Apache-2.0 | https://github.com/vorner/signal-hook |
| signature | 2.2.0 | Apache-2.0 OR MIT | https://github.com/RustCrypto/traits/tree/master/signature |
| simd_cesu8 | 1.1.1 | Apache-2.0 OR MIT | https://github.com/seancroach/simd_cesu8 |
| simd-adler32 | 0.3.8 | MIT | https://github.com/mcountryman/simd-adler32 |
| simdutf8 | 0.1.5 | MIT OR Apache-2.0 | https://github.com/rusticstuff/simdutf8 |
| slab | 0.4.12 | MIT | https://github.com/tokio-rs/slab |
| smallvec | 1.15.1 | MIT OR Apache-2.0 | https://github.com/servo/rust-smallvec |
| socket2 | 0.6.3 | MIT OR Apache-2.0 | https://github.com/rust-lang/socket2 |
| solana-account | 2.2.1 | Apache-2.0 | https://github.com/anza-xyz/agave |
| solana-account-decoder-client-types | 2.3.13 | Apache-2.0 | https://github.com/anza-xyz/agave |
| solana-account-info | 2.3.0 | Apache-2.0 | https://github.com/anza-xyz/solana-sdk |
| solana-address-lookup-table-interface | 2.2.2 | Apache-2.0 | https://github.com/anza-xyz/agave |
| solana-atomic-u64 | 2.2.1 | Apache-2.0 | https://github.com/anza-xyz/agave |
| solana-big-mod-exp | 2.2.1 | Apache-2.0 | https://github.com/anza-xyz/agave |
| solana-bincode | 2.2.1 | Apache-2.0 | https://github.com/anza-xyz/agave |
| solana-blake3-hasher | 2.2.1 | Apache-2.0 | https://github.com/anza-xyz/agave |
| solana-bn254 | 2.2.2 | Apache-2.0 | https://github.com/anza-xyz/solana-sdk |
| solana-borsh | 2.2.1 | Apache-2.0 | https://github.com/anza-xyz/agave |
| solana-clock | 2.2.3 | Apache-2.0 | https://github.com/anza-xyz/solana-sdk |
| solana-commitment-config | 2.2.1 | Apache-2.0 | https://github.com/anza-xyz/agave |
| solana-compute-budget-interface | 2.2.2 | Apache-2.0 | https://github.com/anza-xyz/solana-sdk |
| solana-cpi | 2.2.1 | Apache-2.0 | https://github.com/anza-xyz/agave |
| solana-curve25519 | 2.3.13 | Apache-2.0 | https://github.com/anza-xyz/agave |
| solana-decode-error | 2.3.0 | Apache-2.0 | https://github.com/anza-xyz/solana-sdk |
| solana-define-syscall | 2.3.0 | Apache-2.0 | https://github.com/anza-xyz/solana-sdk |
| solana-derivation-path | 2.2.1 | Apache-2.0 | https://github.com/anza-xyz/agave |
| solana-ed25519-program | 2.2.3 | Apache-2.0 | https://github.com/anza-xyz/solana-sdk |
| solana-epoch-info | 2.2.1 | Apache-2.0 | https://github.com/anza-xyz/agave |
| solana-epoch-rewards | 2.2.1 | Apache-2.0 | https://github.com/anza-xyz/agave |
| solana-epoch-schedule | 2.2.1 | Apache-2.0 | https://github.com/anza-xyz/agave |
| solana-example-mocks | 2.2.1 | Apache-2.0 | https://github.com/anza-xyz/agave |
| solana-feature-gate-interface | 2.2.2 | Apache-2.0 | https://github.com/anza-xyz/solana-sdk |
| solana-feature-set | 2.2.5 | Apache-2.0 | https://github.com/anza-xyz/solana-sdk |
| solana-fee-calculator | 2.2.1 | Apache-2.0 | https://github.com/anza-xyz/agave |
| solana-hash | 2.3.0 | Apache-2.0 | https://github.com/anza-xyz/solana-sdk |
| solana-inflation | 2.2.1 | Apache-2.0 | https://github.com/anza-xyz/agave |
| solana-instruction | 2.3.3 | Apache-2.0 | https://github.com/anza-xyz/solana-sdk |
| solana-instructions-sysvar | 2.2.2 | Apache-2.0 | https://github.com/anza-xyz/solana-sdk |
| solana-keccak-hasher | 2.2.1 | Apache-2.0 | https://github.com/anza-xyz/agave |
| solana-keypair | 2.2.3 | Apache-2.0 | https://github.com/anza-xyz/solana-sdk |
| solana-last-restart-slot | 2.2.1 | Apache-2.0 | https://github.com/anza-xyz/agave |
| solana-loader-v2-interface | 2.2.1 | Apache-2.0 | https://github.com/anza-xyz/agave |
| solana-loader-v3-interface | 5.0.0 | Apache-2.0 | https://github.com/anza-xyz/solana-sdk |
| solana-loader-v4-interface | 2.2.1 | Apache-2.0 | https://github.com/anza-xyz/agave |
| solana-message | 2.4.0 | Apache-2.0 | https://github.com/anza-xyz/solana-sdk |
| solana-msg | 2.2.1 | Apache-2.0 | https://github.com/anza-xyz/agave |
| solana-native-token | 2.3.0 | Apache-2.0 | https://github.com/anza-xyz/solana-sdk |
| solana-nonce | 2.2.1 | Apache-2.0 | https://github.com/anza-xyz/agave |
| solana-nostd-keccak | 0.1.3 | MIT | https://crates.io/crates/solana-nostd-keccak/0.1.3 |
| solana-offchain-message | 2.2.1 | Apache-2.0 | https://github.com/anza-xyz/agave |
| solana-packet | 2.2.1 | Apache-2.0 | https://github.com/anza-xyz/agave |
| solana-precompile-error | 2.2.2 | Apache-2.0 | https://github.com/anza-xyz/solana-sdk |
| solana-program | 2.3.0 | Apache-2.0 | https://github.com/anza-xyz/solana-sdk |
| solana-program-entrypoint | 2.3.0 | Apache-2.0 | https://github.com/anza-xyz/solana-sdk |
| solana-program-error | 2.2.2 | Apache-2.0 | https://github.com/anza-xyz/solana-sdk |
| solana-program-memory | 2.3.1 | Apache-2.0 | https://github.com/anza-xyz/solana-sdk |
| solana-program-option | 2.2.1 | Apache-2.0 | https://github.com/anza-xyz/agave |
| solana-program-pack | 2.2.1 | Apache-2.0 | https://github.com/anza-xyz/agave |
| solana-pubkey | 2.4.0 | Apache-2.0 | https://github.com/anza-xyz/solana-sdk |
| solana-remote-wallet | 2.3.13 | Apache-2.0 | https://github.com/anza-xyz/agave |
| solana-rent | 2.2.1 | Apache-2.0 | https://github.com/anza-xyz/agave |
| solana-reward-info | 2.2.1 | Apache-2.0 | https://github.com/anza-xyz/agave |
| solana-rpc-client | 2.3.13 | Apache-2.0 | https://github.com/anza-xyz/agave |
| solana-rpc-client-api | 2.3.13 | Apache-2.0 | https://github.com/anza-xyz/agave |
| solana-rpc-client-types | 2.3.13 | Apache-2.0 | https://github.com/anza-xyz/agave |
| solana-sanitize | 2.2.1 | Apache-2.0 | https://github.com/anza-xyz/agave |
| solana-sdk-ids | 2.2.1 | Apache-2.0 | https://github.com/anza-xyz/agave |
| solana-sdk-macro | 2.2.1 | Apache-2.0 | https://github.com/anza-xyz/agave |
| solana-secp256k1-recover | 2.2.1 | Apache-2.0 | https://github.com/anza-xyz/agave |
| solana-security-txt | 1.1.2 | MIT OR Apache-2.0 | https://github.com/neodyme-labs/solana-security-txt |
| solana-seed-derivable | 2.2.1 | Apache-2.0 | https://github.com/anza-xyz/agave |
| solana-seed-phrase | 2.2.1 | Apache-2.0 | https://github.com/anza-xyz/agave |
| solana-serde-varint | 2.2.2 | Apache-2.0 | https://github.com/anza-xyz/solana-sdk |
| solana-serialize-utils | 2.2.1 | Apache-2.0 | https://github.com/anza-xyz/agave |
| solana-sha256-hasher | 2.3.0 | Apache-2.0 | https://github.com/anza-xyz/solana-sdk |
| solana-short-vec | 2.2.1 | Apache-2.0 | https://github.com/anza-xyz/agave |
| solana-signature | 2.3.0 | Apache-2.0 | https://github.com/anza-xyz/solana-sdk |
| solana-signer | 2.2.1 | Apache-2.0 | https://github.com/anza-xyz/agave |
| solana-slot-hashes | 2.2.1 | Apache-2.0 | https://github.com/anza-xyz/agave |
| solana-slot-history | 2.2.1 | Apache-2.0 | https://github.com/anza-xyz/agave |
| solana-stable-layout | 2.2.1 | Apache-2.0 | https://github.com/anza-xyz/agave |
| solana-stake-interface | 1.2.1 | Apache-2.0 | https://github.com/solana-program/stake |
| solana-svm-feature-set | 2.3.13 | Apache-2.0 | https://github.com/anza-xyz/agave |
| solana-system-interface | 1.0.0 | Apache-2.0 | https://github.com/solana-program/system |
| solana-sysvar | 2.3.0 | Apache-2.0 | https://github.com/anza-xyz/solana-sdk |
| solana-sysvar-id | 2.2.1 | Apache-2.0 | https://github.com/anza-xyz/agave |
| solana-transaction | 2.2.3 | Apache-2.0 | https://github.com/anza-xyz/solana-sdk |
| solana-transaction-context | 2.3.13 | Apache-2.0 | https://github.com/anza-xyz/agave |
| solana-transaction-error | 2.2.1 | Apache-2.0 | https://github.com/anza-xyz/agave |
| solana-transaction-status-client-types | 2.3.13 | Apache-2.0 | https://github.com/anza-xyz/agave |
| solana-version | 2.3.13 | Apache-2.0 | https://github.com/anza-xyz/agave |
| solana-vote-interface | 2.2.6 | Apache-2.0 | https://github.com/anza-xyz/solana-sdk |
| solana-zk-sdk | 2.3.13 | Apache-2.0 | https://github.com/anza-xyz/agave |
| spki | 0.7.3 | Apache-2.0 OR MIT | https://github.com/RustCrypto/formats/tree/master/spki |
| spl-associated-token-account | 6.0.0 | Apache-2.0 | https://github.com/solana-labs/solana-program-library |
| spl-associated-token-account-client | 2.0.0 | Apache-2.0 | https://github.com/solana-labs/solana-program-library |
| spl-discriminator | 0.4.1 | Apache-2.0 | https://github.com/solana-program/libraries |
| spl-discriminator-derive | 0.2.0 | Apache-2.0 | https://github.com/solana-labs/solana-program-library |
| spl-discriminator-syn | 0.2.1 | Apache-2.0 | https://github.com/solana-program/libraries |
| spl-elgamal-registry | 0.1.1 | Apache-2.0 | https://github.com/solana-program/token-2022 |
| spl-generic-token | 1.0.1 | Apache-2.0 | https://github.com/solana-program/libraries |
| spl-memo | 6.0.0 | Apache-2.0 | https://github.com/solana-labs/solana-program-library |
| spl-pod | 0.5.1 | Apache-2.0 | https://github.com/solana-program/libraries |
| spl-program-error | 0.6.0 | Apache-2.0 | https://github.com/solana-labs/solana-program-library |
| spl-program-error-derive | 0.4.1 | Apache-2.0 | https://github.com/solana-labs/solana-program-library |
| spl-tlv-account-resolution | 0.9.0 | Apache-2.0 | https://github.com/solana-labs/solana-program-library |
| spl-token | 7.0.0 | Apache-2.0 | https://github.com/solana-labs/solana-program-library |
| spl-token-2022 | 6.0.0 | Apache-2.0 | https://github.com/solana-labs/solana-program-library |
| spl-token-2022 | 7.0.0 | Apache-2.0 | https://github.com/solana-program/token-2022 |
| spl-token-2022-interface | 1.0.0 | Apache-2.0 | https://github.com/solana-program/token-2022 |
| spl-token-confidential-transfer-ciphertext-arithmetic | 0.2.1 | Apache-2.0 | https://github.com/solana-program/token-2022 |
| spl-token-confidential-transfer-proof-extraction | 0.2.1 | Apache-2.0 | https://github.com/solana-program/token-2022 |
| spl-token-confidential-transfer-proof-extraction | 0.4.1 | Apache-2.0 | https://github.com/solana-program/token-2022 |
| spl-token-confidential-transfer-proof-generation | 0.2.0 | Apache-2.0 | https://github.com/solana-labs/solana-program-library |
| spl-token-confidential-transfer-proof-generation | 0.3.0 | Apache-2.0 | https://github.com/solana-program/token-2022 |
| spl-token-confidential-transfer-proof-generation | 0.4.1 | Apache-2.0 | https://github.com/solana-program/token-2022 |
| spl-token-group-interface | 0.5.0 | Apache-2.0 | https://github.com/solana-labs/solana-program-library |
| spl-token-group-interface | 0.6.0 | Apache-2.0 | https://github.com/solana-program/token-group |
| spl-token-metadata-interface | 0.6.0 | Apache-2.0 | https://github.com/solana-labs/solana-program-library |
| spl-token-metadata-interface | 0.7.0 | Apache-2.0 | https://github.com/solana-program/token-metadata |
| spl-transfer-hook-interface | 0.9.0 | Apache-2.0 | https://github.com/solana-labs/solana-program-library |
| spl-type-length-value | 0.7.0 | Apache-2.0 | https://github.com/solana-labs/solana-program-library |
| spl-type-length-value | 0.8.0 | Apache-2.0 | https://github.com/solana-program/libraries |
| stable_deref_trait | 1.2.1 | MIT OR Apache-2.0 | https://github.com/storyyeller/stable_deref_trait |
| static_assertions | 1.1.0 | MIT OR Apache-2.0 | https://github.com/nvzqz/static-assertions-rs |
| strsim | 0.11.1 | MIT | https://github.com/rapidfuzz/strsim-rs |
| strum | 0.26.3 | MIT | https://github.com/Peternator7/strum |
| strum_macros | 0.26.4 | MIT | https://github.com/Peternator7/strum |
| subtle | 2.6.1 | BSD-3-Clause | https://github.com/dalek-cryptography/subtle |
| syn | 1.0.109 | MIT OR Apache-2.0 | https://github.com/dtolnay/syn |
| syn | 2.0.117 | MIT OR Apache-2.0 | https://github.com/dtolnay/syn |
| sync_wrapper | 1.0.2 | Apache-2.0 | https://github.com/Actyx/sync_wrapper |
| synstructure | 0.13.2 | MIT | https://github.com/mystor/synstructure |
| system-configuration | 0.7.0 | MIT OR Apache-2.0 | https://github.com/mullvad/system-configuration-rs |
| system-configuration-sys | 0.6.0 | MIT OR Apache-2.0 | https://github.com/mullvad/system-configuration-rs |
| tap | 1.0.1 | MIT | https://github.com/myrrlyn/tap |
| tempfile | 3.27.0 | MIT OR Apache-2.0 | https://github.com/Stebalien/tempfile |
| thiserror | 1.0.69 | MIT OR Apache-2.0 | https://github.com/dtolnay/thiserror |
| thiserror | 2.0.18 | MIT OR Apache-2.0 | https://github.com/dtolnay/thiserror |
| thiserror-impl | 1.0.69 | MIT OR Apache-2.0 | https://github.com/dtolnay/thiserror |
| thiserror-impl | 2.0.18 | MIT OR Apache-2.0 | https://github.com/dtolnay/thiserror |
| tinystr | 0.8.2 | Unicode-3.0 | https://github.com/unicode-org/icu4x |
| tinyvec | 1.10.0 | Zlib OR Apache-2.0 OR MIT | https://github.com/Lokathor/tinyvec |
| tinyvec_macros | 0.1.1 | MIT OR Apache-2.0 OR Zlib | https://github.com/Soveu/tinyvec_macros |
| tokio | 1.50.0 | MIT | https://github.com/tokio-rs/tokio |
| tokio-macros | 2.6.1 | MIT | https://github.com/tokio-rs/tokio |
| tokio-native-tls | 0.3.1 | MIT | https://github.com/tokio-rs/tls |
| tokio-rustls | 0.26.4 | MIT OR Apache-2.0 | https://github.com/rustls/tokio-rustls |
| tokio-util | 0.7.18 | MIT | https://github.com/tokio-rs/tokio |
| toml | 0.5.11 | MIT/Apache-2.0 | https://github.com/toml-rs/toml |
| toml | 1.0.6+spec-1.1.0 | MIT OR Apache-2.0 | https://github.com/toml-rs/toml |
| toml_datetime | 1.0.0+spec-1.1.0 | MIT OR Apache-2.0 | https://github.com/toml-rs/toml |
| toml_edit | 0.25.4+spec-1.1.0 | MIT OR Apache-2.0 | https://github.com/toml-rs/toml |
| toml_parser | 1.0.9+spec-1.1.0 | MIT OR Apache-2.0 | https://github.com/toml-rs/toml |
| toml_writer | 1.1.1+spec-1.1.0 | MIT OR Apache-2.0 | https://github.com/toml-rs/toml |
| tower | 0.5.3 | MIT | https://github.com/tower-rs/tower |
| tower-http | 0.6.8 | MIT | https://github.com/tower-rs/tower-http |
| tower-layer | 0.3.3 | MIT | https://github.com/tower-rs/tower |
| tower-service | 0.3.3 | MIT | https://github.com/tower-rs/tower |
| tracing | 0.1.44 | MIT | https://github.com/tokio-rs/tracing |
| tracing-attributes | 0.1.31 | MIT | https://github.com/tokio-rs/tracing |
| tracing-core | 0.1.36 | MIT | https://github.com/tokio-rs/tracing |
| try-lock | 0.2.5 | MIT | https://github.com/seanmonstar/try-lock |
| typed-path | 0.12.3 | MIT OR Apache-2.0 | https://github.com/chipsenkbeil/typed-path |
| typenum | 1.19.0 | MIT OR Apache-2.0 | https://github.com/paholg/typenum |
| unicode-ident | 1.0.24 | (MIT OR Apache-2.0) AND Unicode-3.0 | https://github.com/dtolnay/unicode-ident |
| unicode-segmentation | 1.13.3 | MIT OR Apache-2.0 | https://github.com/unicode-rs/unicode-segmentation |
| unicode-truncate | 1.1.0 | MIT OR Apache-2.0 | https://github.com/Aetf/unicode-truncate |
| unicode-width | 0.1.14 | MIT OR Apache-2.0 | https://github.com/unicode-rs/unicode-width |
| unicode-width | 0.2.0 | MIT OR Apache-2.0 | https://github.com/unicode-rs/unicode-width |
| unicode-xid | 0.2.6 | MIT OR Apache-2.0 | https://github.com/unicode-rs/unicode-xid |
| universal-hash | 0.5.1 | MIT OR Apache-2.0 | https://github.com/RustCrypto/traits |
| untrusted | 0.9.0 | ISC | https://github.com/briansmith/untrusted |
| uriparse | 0.6.4 | MIT | https://github.com/sgodwincs/uriparse-rs |
| url | 2.5.8 | MIT OR Apache-2.0 | https://github.com/servo/rust-url |
| utf8_iter | 1.0.4 | Apache-2.0 OR MIT | https://github.com/hsivonen/utf8_iter |
| utf8parse | 0.2.2 | Apache-2.0 OR MIT | https://github.com/alacritty/vte |
| vcpkg | 0.2.15 | MIT/Apache-2.0 | https://github.com/mcgoo/vcpkg-rs |
| version_check | 0.9.5 | MIT/Apache-2.0 | https://github.com/SergioBenitez/version_check |
| walkdir | 2.5.0 | Unlicense/MIT | https://github.com/BurntSushi/walkdir |
| want | 0.3.1 | MIT | https://github.com/seanmonstar/want |
| wasi | 0.11.1+wasi-snapshot-preview1 | Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT | https://github.com/bytecodealliance/wasi |
| wasi | 0.9.0+wasi-snapshot-preview1 | Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT | https://github.com/bytecodealliance/wasi |
| wasip2 | 1.0.2+wasi-0.2.9 | Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT | https://github.com/bytecodealliance/wasi-rs |
| wasip3 | 0.4.0+wasi-0.3.0-rc-2026-01-06 | Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT | https://github.com/bytecodealliance/wasi-rs |
| wasm-bindgen | 0.2.114 | MIT OR Apache-2.0 | https://github.com/wasm-bindgen/wasm-bindgen |
| wasm-bindgen-futures | 0.4.64 | MIT OR Apache-2.0 | https://github.com/wasm-bindgen/wasm-bindgen/tree/master/crates/futures |
| wasm-bindgen-macro | 0.2.114 | MIT OR Apache-2.0 | https://github.com/wasm-bindgen/wasm-bindgen/tree/master/crates/macro |
| wasm-bindgen-macro-support | 0.2.114 | MIT OR Apache-2.0 | https://github.com/wasm-bindgen/wasm-bindgen/tree/master/crates/macro-support |
| wasm-bindgen-shared | 0.2.114 | MIT OR Apache-2.0 | https://github.com/wasm-bindgen/wasm-bindgen/tree/master/crates/shared |
| wasm-encoder | 0.244.0 | Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT | https://github.com/bytecodealliance/wasm-tools/tree/main/crates/wasm-encoder |
| wasm-metadata | 0.244.0 | Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT | https://github.com/bytecodealliance/wasm-tools/tree/main/crates/wasm-metadata |
| wasm-streams | 0.5.0 | MIT OR Apache-2.0 | https://github.com/MattiasBuelens/wasm-streams/ |
| wasmparser | 0.244.0 | Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT | https://github.com/bytecodealliance/wasm-tools/tree/main/crates/wasmparser |
| web-sys | 0.3.91 | MIT OR Apache-2.0 | https://github.com/wasm-bindgen/wasm-bindgen/tree/master/crates/web-sys |
| web-time | 1.1.0 | MIT OR Apache-2.0 | https://github.com/daxpedda/web-time |
| webpki-root-certs | 1.0.8 | CDLA-Permissive-2.0 | https://github.com/rustls/webpki-roots |
| webpki-roots | 1.0.6 | CDLA-Permissive-2.0 | https://github.com/rustls/webpki-roots |
| winapi | 0.3.9 | MIT/Apache-2.0 | https://github.com/retep998/winapi-rs |
| winapi-i686-pc-windows-gnu | 0.4.0 | MIT/Apache-2.0 | https://github.com/retep998/winapi-rs |
| winapi-util | 0.1.11 | Unlicense OR MIT | https://github.com/BurntSushi/winapi-util |
| winapi-x86_64-pc-windows-gnu | 0.4.0 | MIT/Apache-2.0 | https://github.com/retep998/winapi-rs |
| windows_aarch64_gnullvm | 0.52.6 | MIT OR Apache-2.0 | https://github.com/microsoft/windows-rs |
| windows_aarch64_msvc | 0.52.6 | MIT OR Apache-2.0 | https://github.com/microsoft/windows-rs |
| windows_i686_gnu | 0.52.6 | MIT OR Apache-2.0 | https://github.com/microsoft/windows-rs |
| windows_i686_gnullvm | 0.52.6 | MIT OR Apache-2.0 | https://github.com/microsoft/windows-rs |
| windows_i686_msvc | 0.52.6 | MIT OR Apache-2.0 | https://github.com/microsoft/windows-rs |
| windows_x86_64_gnu | 0.52.6 | MIT OR Apache-2.0 | https://github.com/microsoft/windows-rs |
| windows_x86_64_gnullvm | 0.52.6 | MIT OR Apache-2.0 | https://github.com/microsoft/windows-rs |
| windows_x86_64_msvc | 0.52.6 | MIT OR Apache-2.0 | https://github.com/microsoft/windows-rs |
| windows-core | 0.62.2 | MIT OR Apache-2.0 | https://github.com/microsoft/windows-rs |
| windows-implement | 0.60.2 | MIT OR Apache-2.0 | https://github.com/microsoft/windows-rs |
| windows-interface | 0.59.3 | MIT OR Apache-2.0 | https://github.com/microsoft/windows-rs |
| windows-link | 0.2.1 | MIT OR Apache-2.0 | https://github.com/microsoft/windows-rs |
| windows-registry | 0.6.1 | MIT OR Apache-2.0 | https://github.com/microsoft/windows-rs |
| windows-result | 0.4.1 | MIT OR Apache-2.0 | https://github.com/microsoft/windows-rs |
| windows-strings | 0.5.1 | MIT OR Apache-2.0 | https://github.com/microsoft/windows-rs |
| windows-sys | 0.52.0 | MIT OR Apache-2.0 | https://github.com/microsoft/windows-rs |
| windows-sys | 0.59.0 | MIT OR Apache-2.0 | https://github.com/microsoft/windows-rs |
| windows-sys | 0.61.2 | MIT OR Apache-2.0 | https://github.com/microsoft/windows-rs |
| windows-targets | 0.52.6 | MIT OR Apache-2.0 | https://github.com/microsoft/windows-rs |
| winnow | 0.7.15 | MIT | https://github.com/winnow-rs/winnow |
| winresource | 0.1.31 | MIT | https://github.com/BenjaminRi/winresource |
| wit-bindgen | 0.51.0 | Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT | https://github.com/bytecodealliance/wit-bindgen |
| wit-bindgen-core | 0.51.0 | Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT | https://github.com/bytecodealliance/wit-bindgen |
| wit-bindgen-rust | 0.51.0 | Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT | https://github.com/bytecodealliance/wit-bindgen |
| wit-bindgen-rust-macro | 0.51.0 | Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT | https://github.com/bytecodealliance/wit-bindgen |
| wit-component | 0.244.0 | Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT | https://github.com/bytecodealliance/wasm-tools/tree/main/crates/wit-component |
| wit-parser | 0.244.0 | Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT | https://github.com/bytecodealliance/wasm-tools/tree/main/crates/wit-parser |
| writeable | 0.6.2 | Unicode-3.0 | https://github.com/unicode-org/icu4x |
| wyz | 0.5.1 | MIT | https://github.com/myrrlyn/wyz |
| yaml-rust2 | 0.11.0 | MIT OR Apache-2.0 | https://github.com/Ethiraric/yaml-rust2 |
| yoke | 0.8.1 | Unicode-3.0 | https://github.com/unicode-org/icu4x |
| yoke-derive | 0.8.1 | Unicode-3.0 | https://github.com/unicode-org/icu4x |
| zerocopy | 0.8.42 | BSD-2-Clause OR Apache-2.0 OR MIT | https://github.com/google/zerocopy |
| zerocopy-derive | 0.8.42 | BSD-2-Clause OR Apache-2.0 OR MIT | https://github.com/google/zerocopy |
| zerofrom | 0.1.6 | Unicode-3.0 | https://github.com/unicode-org/icu4x |
| zerofrom-derive | 0.1.6 | Unicode-3.0 | https://github.com/unicode-org/icu4x |
| zeroize | 1.8.2 | Apache-2.0 OR MIT | https://github.com/RustCrypto/utils |
| zeroize_derive | 1.4.3 | Apache-2.0 OR MIT | https://github.com/RustCrypto/utils/tree/master/zeroize/derive |
| zerotrie | 0.2.3 | Unicode-3.0 | https://github.com/unicode-org/icu4x |
| zerovec | 0.11.5 | Unicode-3.0 | https://github.com/unicode-org/icu4x |
| zerovec-derive | 0.11.2 | Unicode-3.0 | https://github.com/unicode-org/icu4x |
| zip | 8.6.0 | MIT | https://github.com/zip-rs/zip2 |
| zmij | 1.0.21 | MIT | https://github.com/dtolnay/zmij |
| zstd | 0.13.3 | MIT | https://github.com/gyscos/zstd-rs |
| zstd-safe | 7.2.4 | MIT OR Apache-2.0 | https://github.com/gyscos/zstd-rs |
| zstd-sys | 2.0.16+zstd.1.5.7 | MIT/Apache-2.0 | https://github.com/gyscos/zstd-rs |
