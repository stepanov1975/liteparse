## Maturin

Maturin has **first-class support** for `cargo-zigbuild` via the `--zig` flag:

```bash
maturin build --release --target x86_64-unknown-linux-musl --zig
maturin build --release --target aarch64-unknown-linux-musl --zig
```

That's it. Maturin detects `cargo-zigbuild` is installed and routes through it automatically when `--zig` is passed. You just need:

```bash
pip install maturin
cargo install cargo-zigbuild
# and the targets:
rustup target add x86_64-unknown-linux-musl aarch64-unknown-linux-musl
```

In a CI matrix (GitHub Actions example):
```yaml
- name: Build musl wheels
  run: |
    maturin build --release --target ${{ matrix.target }} --zig
  strategy:
    matrix:
      target: [x86_64-unknown-linux-musl, aarch64-unknown-linux-musl]
```

## napi-rs

Actually, the **real pattern** for napi is to override the cargo command via the `CARGO` env var or use napi's `--cargo-cwd` + a wrapper script. The cleanest approach in practice:

```bash
# Tell napi-rs to use cargo-zigbuild instead of cargo
CARGO=cargo-zigbuild npx @napi-rs/cli build \
  --platform \
  --release \
  --target x86_64-unknown-linux-musl
```

`napi-rs` internally shells out to whatever `$CARGO` points to, and `cargo-zigbuild` accepts the same interface as `cargo build`, so this works cleanly.

## The even cleaner napi path: use their Docker images

The `@napi-rs/cli` team maintains cross-compilation Docker images that have Zig + musl toolchains pre-configured:

```bash
docker run --rm -v $(pwd):/build \
  ghcr.io/napi-rs/napi-rs/nodejs-rust:lts-alpine \
  sh -c "cd /build && npx @napi-rs/cli build --release --target x86_64-unknown-linux-musl"
```

Their [CI template](https://github.com/napi-rs/package-template) scaffolds all of this out of the box if you use `napi new`.

## Summary

| Tool | musl + Zig support |
|---|---|
| maturin | Native `--zig` flag, trivial |
| napi-rs | `CARGO=cargo-zigbuild` env var, or use their Docker images |

For maturin, `--zig` is the blessed path. For napi-rs, the `CARGO` env var override or their Docker-based CI template is the practical answer.
