# Rust dependency notices

These notices accompany the exact registry packages in `Cargo.lock`. The MIT
alternative is selected where a package offers a choice. The additional
Unicode-3.0 notice is retained for `unicode-ident`. License files are copied
without alteration from each published crate archive; the versioned crates.io
links below identify their public source. Cargo verifies archive checksums
against `Cargo.lock`.

Build dependencies are included for completeness even though their code runs
on the build host and is not linked into the Switch NRO. Target dependencies
may have unused code removed by the linker. `libc` is locked for other
platforms but is not selected by this Switch dependency graph.

| Package and source | Use | License text |
| --- | --- | --- |
| [block-buffer 0.10.4](https://crates.io/crates/block-buffer/0.10.4) | Target dependency | [LICENSE-MIT](block-buffer-0.10.4/LICENSE-MIT) |
| [cfg-if 1.0.5](https://crates.io/crates/cfg-if/1.0.5) | Target dependency | [LICENSE-MIT](cfg-if-1.0.5/LICENSE-MIT) |
| [cpufeatures 0.2.17](https://crates.io/crates/cpufeatures/0.2.17) | Target dependency | [LICENSE-MIT](cpufeatures-0.2.17/LICENSE-MIT) |
| [crypto-common 0.1.7](https://crates.io/crates/crypto-common/0.1.7) | Target dependency | [LICENSE-MIT](crypto-common-0.1.7/LICENSE-MIT) |
| [digest 0.10.7](https://crates.io/crates/digest/0.10.7) | Target dependency | [LICENSE-MIT](digest-0.10.7/LICENSE-MIT) |
| [generic-array 0.14.7](https://crates.io/crates/generic-array/0.14.7) | Target dependency | [LICENSE](generic-array-0.14.7/LICENSE) |
| [itoa 1.0.18](https://crates.io/crates/itoa/1.0.18) | Target dependency | [LICENSE-MIT](itoa-1.0.18/LICENSE-MIT) |
| [libc 0.2.190](https://crates.io/crates/libc/0.2.190) | Platform-conditional; not selected by the Switch target | [LICENSE-MIT](libc-0.2.190/LICENSE-MIT) |
| [memchr 2.8.3](https://crates.io/crates/memchr/2.8.3) | Target dependency | [LICENSE-MIT](memchr-2.8.3/LICENSE-MIT) |
| [proc-macro2 1.0.107](https://crates.io/crates/proc-macro2/1.0.107) | Build dependency; not linked into the NRO | [LICENSE-MIT](proc-macro2-1.0.107/LICENSE-MIT) |
| [quote 1.0.47](https://crates.io/crates/quote/1.0.47) | Build dependency; not linked into the NRO | [LICENSE-MIT](quote-1.0.47/LICENSE-MIT) |
| [serde 1.0.229](https://crates.io/crates/serde/1.0.229) | Target dependency | [LICENSE-MIT](serde-1.0.229/LICENSE-MIT) |
| [serde_core 1.0.229](https://crates.io/crates/serde_core/1.0.229) | Target dependency | [LICENSE-MIT](serde_core-1.0.229/LICENSE-MIT) |
| [serde_derive 1.0.229](https://crates.io/crates/serde_derive/1.0.229) | Build dependency; not linked into the NRO | [LICENSE-MIT](serde_derive-1.0.229/LICENSE-MIT) |
| [serde_json 1.0.154](https://crates.io/crates/serde_json/1.0.154) | Target dependency | [LICENSE-MIT](serde_json-1.0.154/LICENSE-MIT) |
| [sha2 0.10.9](https://crates.io/crates/sha2/0.10.9) | Target dependency | [LICENSE-MIT](sha2-0.10.9/LICENSE-MIT) |
| [syn 3.0.9](https://crates.io/crates/syn/3.0.9) | Build dependency; not linked into the NRO | [LICENSE-MIT](syn-3.0.9/LICENSE-MIT) |
| [typenum 1.20.1](https://crates.io/crates/typenum/1.20.1) | Target dependency | [LICENSE-MIT](typenum-1.20.1/LICENSE-MIT) |
| [unicode-ident 1.0.27](https://crates.io/crates/unicode-ident/1.0.27) | Build dependency; not linked into the NRO | [LICENSE-MIT](unicode-ident-1.0.27/LICENSE-MIT), [LICENSE-UNICODE](unicode-ident-1.0.27/LICENSE-UNICODE) |
| [version_check 0.9.5](https://crates.io/crates/version_check/0.9.5) | Build dependency; not linked into the NRO | [LICENSE-MIT](version_check-0.9.5/LICENSE-MIT) |
| [zmij 1.0.23](https://crates.io/crates/zmij/1.0.23) | Target dependency | [LICENSE-MIT](zmij-1.0.23/LICENSE-MIT) |

## Rust runtime

The Switch build uses `no_std` plus `alloc`, rebuilding `core`, `alloc`, and
`compiler_builtins` from `nightly-2026-10-01`: Rust 1.101.0-nightly,
commit [`21b707e3f97e0b522ebd2f277a862339625ad83f`](https://github.com/rust-lang/rust/tree/21b707e3f97e0b522ebd2f277a862339625ad83f), dated 2026-09-30.
The runtime notices here correspond to that source revision, not a separate
standard-library binary. The desktop build also uses the Rust standard
library under the same MIT license alternative.

- `core` and `alloc`: [MIT license](runtime/LICENSE-MIT), retrieved from
  [the matching upstream Rust commit](https://github.com/rust-lang/rust/blob/21b707e3f97e0b522ebd2f277a862339625ad83f/LICENSE-MIT).
- Architecture intrinsics included by `core`: [stdarch MIT license](runtime/stdarch-LICENSE-MIT),
  from [`library/stdarch/crates/core_arch/LICENSE-MIT`](https://github.com/rust-lang/rust/tree/21b707e3f97e0b522ebd2f277a862339625ad83f/library/stdarch/crates/core_arch/LICENSE-MIT).
- Portable SIMD sources included by `core`: [MIT license](runtime/portable-simd-LICENSE-MIT),
  from [`library/portable-simd/crates/core_simd/LICENSE-MIT`](https://github.com/rust-lang/rust/tree/21b707e3f97e0b522ebd2f277a862339625ad83f/library/portable-simd/crates/core_simd/LICENSE-MIT).
- `compiler_builtins` 0.1.160: [complete license and notices](runtime/compiler-builtins-LICENSE.txt),
  from [`library/compiler-builtins/LICENSE.txt`](https://github.com/rust-lang/rust/tree/21b707e3f97e0b522ebd2f277a862339625ad83f/library/compiler-builtins/LICENSE.txt).
  This retains the compound MIT and Apache-2.0-with-LLVM-exception terms,
  including compiler-rt attribution; it is not treated as an MIT-only choice.
- Bundled `libm` 0.2.16: [complete license and notices](runtime/libm-LICENSE.txt),
  from [`library/compiler-builtins/libm/LICENSE.txt`](https://github.com/rust-lang/rust/tree/21b707e3f97e0b522ebd2f277a862339625ad83f/library/compiler-builtins/libm/LICENSE.txt).
- [Additional source notices](runtime/SOURCE-NOTICES.txt) retain copyright,
  permission, and disclaimer blocks from the runtime sources, including
  FreeBSD, Sun Microsystems, CORE-MATH, and Rust `memchr` attributions. Each
  block identifies its upstream source. The full runtime notice set is
  included even when particular functions are removed from the final binary.

When changing `Cargo.lock` or the pinned nightly, refresh these files from the
matching public sources before distributing a release.
