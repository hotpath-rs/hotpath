//! Deterministic One Billion Row Challenge dataset: `<station>;<temp>\n` rows,
//! temperatures with exactly one decimal in [-99.9, 99.9]. Integer arithmetic
//! only, so a given `(rows, seed)` produces byte-identical output on every CPU.

use std::io::{self, Write};

use crate::stations::STATIONS;

/// Seed of the benchmark dataset. Changing it changes the committed SHA-256 and
/// expected output.
pub const SEED: u64 = 0x1b2c_2024;

/// splitmix64: tiny, fast and fully specified, so no `rand` dependency.
struct SplitMix64(u64);

impl SplitMix64 {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }
}

/// Writes `rows` rows generated from `seed` to `out`.
pub fn generate(rows: u64, seed: u64, out: &mut impl Write) -> io::Result<()> {
    let mut rng = SplitMix64(seed);
    let mut line = Vec::with_capacity(64);
    for _ in 0..rows {
        let r = rng.next();
        let (name, mean) = STATIONS[(r % STATIONS.len() as u64) as usize];
        // Sum of four uniform draws in [-87, 87] tenths: roughly normal with a
        // standard deviation of about 10 degrees, like the original generator.
        let mut noise = 0i32;
        for shift in [8, 20, 32, 44] {
            noise += ((r >> shift) % 175) as i32 - 87;
        }
        let temp = (i32::from(mean) + noise).clamp(-999, 999);

        line.clear();
        line.extend_from_slice(name.as_bytes());
        line.push(b';');
        write_tenths(&mut line, temp);
        line.push(b'\n');
        out.write_all(&line)?;
    }
    Ok(())
}

/// Appends `tenths` as a decimal with exactly one fractional digit.
pub(crate) fn write_tenths(out: &mut Vec<u8>, tenths: i32) {
    if tenths < 0 {
        out.push(b'-');
    }
    let abs = tenths.unsigned_abs();
    out.extend_from_slice((abs / 10).to_string().as_bytes());
    out.push(b'.');
    out.push(b'0' + (abs % 10) as u8);
}
