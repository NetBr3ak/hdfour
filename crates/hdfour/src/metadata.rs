//! Byte budgets for retained metadata and temporary parser caches.
#![deny(
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic
)]

use std::sync::atomic::{AtomicU64, Ordering};

use crate::read::{allocation_error, allocation_size, malformed};
use crate::{Result, dd};

#[derive(Debug)]
pub(crate) struct Budget {
    used: AtomicU64,
    max: u64,
}

impl Budget {
    pub fn used(&self) -> u64 {
        self.used.load(Ordering::Relaxed)
    }

    pub fn new(max: u64) -> Self {
        Self {
            used: AtomicU64::new(0),
            max,
        }
    }

    #[allow(
        deprecated,
        reason = "fetch_update preserves the Rust 1.85 minimum version"
    )]
    pub fn charge(&self, bytes: u64) -> Result<()> {
        dd::check_limit("metadata_bytes", bytes, self.max)?;
        allocation_size(bytes)?;
        self.used
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |previous| {
                previous
                    .checked_add(bytes)
                    .filter(|total| *total <= self.max)
            })
            .map_err(|previous| match previous.checked_add(bytes) {
                Some(total) => crate::Error::LimitExceeded {
                    limit: "metadata_bytes",
                    value: total,
                    max: self.max,
                },
                None => malformed("metadata byte count overflow"),
            })?;
        Ok(())
    }

    pub fn items<T>(&self, count: usize) -> Result<()> {
        let bytes = count
            .checked_mul(std::mem::size_of::<T>())
            .ok_or_else(|| allocation_error(u64::MAX))?;
        self.charge(bytes as u64)
    }

    pub fn text(&self, value: &str) -> Result<String> {
        self.charge(value.len() as u64)?;
        let mut text = String::new();
        text.try_reserve_exact(value.len())
            .map_err(|_| allocation_error(value.len() as u64))?;
        text.push_str(value);
        Ok(text)
    }

    pub fn latin1(&self, bytes: &[u8]) -> Result<String> {
        let length = bytes.iter().try_fold(0_u64, |total, byte| {
            total
                .checked_add(char::from(*byte).len_utf8() as u64)
                .ok_or_else(|| malformed("metadata text length overflow"))
        })?;
        self.charge(length)?;
        let mut text = String::new();
        text.try_reserve_exact(allocation_size(length)?)
            .map_err(|_| allocation_error(length))?;
        text.extend(bytes.iter().map(|byte| char::from(*byte)));
        Ok(text)
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::arithmetic_side_effects)]
mod tests {
    use super::*;
    use std::sync::Barrier;

    #[test]
    fn sq10_concurrent_charges_do_not_lose_updates() {
        let budget = Budget::new(4000);
        let start = Barrier::new(8);
        let charged = std::thread::scope(|scope| {
            let workers: Vec<_> = (0..8)
                .map(|_| {
                    scope.spawn(|| {
                        start.wait();
                        (0..1000).filter(|_| budget.charge(1).is_ok()).count() as u64
                    })
                })
                .collect();
            workers
                .into_iter()
                .map(|worker| worker.join().expect("worker"))
                .sum::<u64>()
        });
        assert_eq!(charged, 4000);
        assert_eq!(budget.used(), charged);
    }
}
