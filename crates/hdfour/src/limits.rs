//! Per-file resource budgets; all sizes are in bytes unless named otherwise.

/// Resource ceilings applied while parsing metadata and reading arrays.
#[derive(Debug, Clone, PartialEq)]
pub struct Limits {
    pub max_descriptors: u64,
    pub max_dd_blocks: u64,
    pub max_group_depth: u64,
    pub max_rank: u64,
    pub max_array_bytes: u64,
    pub max_attribute_bytes: u64,
    pub max_metadata_bytes: u64,
    pub max_chunks: u64,
    pub max_chunk_header_bytes: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_descriptors: 4_194_304,
            max_dd_blocks: 65_536,
            max_group_depth: 64,
            max_rank: 32,
            max_array_bytes: 2_147_483_648,
            max_attribute_bytes: 67_108_864,
            max_metadata_bytes: 67_108_864,
            max_chunks: 1_048_576,
            max_chunk_header_bytes: 256,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sq7_metadata_budget_defaults_to_64_mib() {
        assert_eq!(Limits::default().max_metadata_bytes, 67_108_864);
    }

    #[test]
    fn specification_defaults() {
        let limits = Limits::default();
        assert_eq!(limits.max_descriptors, 4_194_304);
        assert_eq!(limits.max_dd_blocks, 65_536);
        assert_eq!(limits.max_group_depth, 64);
        assert_eq!(limits.max_rank, 32);
        assert_eq!(limits.max_array_bytes, 2_147_483_648);
        assert_eq!(limits.max_attribute_bytes, 67_108_864);
        assert_eq!(limits.max_chunks, 1_048_576);
        assert_eq!(limits.max_chunk_header_bytes, 256);
    }
}
