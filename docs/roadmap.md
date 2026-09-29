# Roadmap

- Color effects use exact scalar lanes; evaluate stable-Rust SIMD alternatives against the per-dot reference and measure before replacing the hot path.
- Compressed rewind entries differ byte-wise from ZiGBA's while remaining mutually decodable; measure miniz_oxide level 1 against Zig `level_1` on representative ~0.5 MB states for compression ratio and encoding speed.
