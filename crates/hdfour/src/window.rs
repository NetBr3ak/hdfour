//! Rectangular selections in C storage order.
#![deny(
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic
)]

use crate::read::malformed;
use crate::{Error, Result};

pub(crate) struct Window<'a> {
    shape: &'a [u64],
    pub start: &'a [u64],
    pub count: &'a [u64],
}

impl<'a> Window<'a> {
    pub fn new(shape: &'a [u64], start: &'a [u64], count: &'a [u64]) -> Result<Self> {
        if start.len() != shape.len() || count.len() != shape.len() {
            return Err(Error::InvalidSelection {
                reason: "start and count must match the dataset rank",
            });
        }
        for ((size, origin), length) in shape.iter().zip(start).zip(count) {
            if origin.checked_add(*length).is_none_or(|end| end > *size) {
                return Err(Error::InvalidSelection {
                    reason: "window extends beyond the dataset",
                });
            }
        }
        Ok(Self {
            shape,
            start,
            count,
        })
    }

    /// Merge complete trailing axes so adjacent rows require a single source read.
    pub fn read_runs(
        &self,
        width: usize,
        output: &mut [u8],
        mut read: impl FnMut(u64, &mut [u8]) -> Result<()>,
    ) -> Result<()> {
        if output.is_empty() {
            return Ok(());
        }
        let overflow = || malformed("window byte offset overflow");
        let mut span = width as u64;
        let mut prefix = self.shape.len();
        for (&size, &selected) in self.shape.iter().zip(self.count).rev() {
            prefix = prefix.checked_sub(1).ok_or_else(overflow)?;
            span = span.checked_mul(selected).ok_or_else(overflow)?;
            if selected != size {
                break;
            }
        }
        let first = self
            .shape
            .iter()
            .zip(self.start)
            .try_fold(0_u64, |offset, (&size, &start)| {
                offset
                    .checked_mul(size)
                    .and_then(|n| n.checked_add(start))
                    .ok_or_else(overflow)
            })?
            .checked_mul(width as u64)
            .ok_or_else(overflow)?;
        let mut stride = width as u64;
        let mut axes = Vec::with_capacity(prefix);
        for (axis, (&size, &count)) in self.shape.iter().zip(self.count).enumerate().rev() {
            if axis < prefix {
                axes.push((count, stride));
            }
            stride = stride.checked_mul(size).ok_or_else(overflow)?;
        }
        let span = usize::try_from(span).map_err(|_| overflow())?;
        for (run, dest) in output.chunks_exact_mut(span).enumerate() {
            let mut ordinal = run as u64;
            let mut offset = first;
            for &(count, stride) in &axes {
                let coordinate = ordinal.checked_rem(count).ok_or_else(overflow)?;
                offset = coordinate
                    .checked_mul(stride)
                    .and_then(|delta| offset.checked_add(delta))
                    .ok_or_else(overflow)?;
                ordinal = ordinal.checked_div(count).ok_or_else(overflow)?;
            }
            read(offset, dest)?;
        }
        Ok(())
    }
}
