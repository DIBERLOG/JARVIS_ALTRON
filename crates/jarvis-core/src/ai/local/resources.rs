//! Memory estimation for the local runtime.
//!
//! This is a **heuristic**, not a calculation. Weights are estimated from the
//! model file size, the KV cache from a documented per-token figure for an
//! 8B-class model, and a fixed allowance covers compute buffers. GPU memory is
//! not estimated at all: usage depends on the GPU, the driver, and the llama.cpp
//! backend, so the report says so instead of guessing.
//!
//! Nothing here is measured remotely or reported anywhere: the numbers come from
//! the local machine and stay in the local process.

use serde::Serialize;

use super::config::LocalAiConfig;
use super::model::CheckLevel;
use super::model::ValidationIssue;

/// Estimated KV cache bytes per context token for an 8B-class model.
///
/// 2 (keys and values) * 36 layers * 8 KV heads * 128 head dimension * 2 bytes.
/// Real models differ, which is why this is documented as an estimate.
pub const KV_BYTES_PER_TOKEN_ESTIMATE: u64 = 2 * 36 * 8 * 128 * 2;

/// Fixed allowance for compute buffers and runtime overhead.
pub const RUNTIME_OVERHEAD_BYTES: u64 = 512 * 1024 * 1024;

/// Memory headroom below which a warning is raised.
pub const HEADROOM_WARNING_RATIO: f64 = 1.25;

/// What the user should know before starting the server.
#[derive(Clone, Debug, Serialize)]
pub struct ResourceEstimate {
    pub model_size_bytes: u64,
    pub context_size: u32,
    pub kv_cache_bytes: u64,
    pub overhead_bytes: u64,
    pub estimated_need_bytes: u64,
    pub available_ram_bytes: u64,
    pub total_ram_bytes: u64,
    /// Positive when the estimate fits, negative when it does not.
    pub headroom_bytes: i64,
    pub level: CheckLevel,
    pub gpu_layers_requested: u32,
    /// Human-readable, content-free notes for the interface.
    pub notes: Vec<String>,
    pub issues: Vec<ValidationIssue>,
}

impl ResourceEstimate {
    pub fn is_blocked(&self) -> bool {
        self.level == CheckLevel::Blocked
    }
}

/// Probes available memory on this machine, in bytes.
pub fn available_memory_bytes() -> u64 {
    let mut system = sysinfo::System::new();
    system.refresh_memory();
    system.available_memory()
}

/// Total physical memory on this machine, in bytes.
pub fn total_memory_bytes() -> u64 {
    let mut system = sysinfo::System::new();
    system.refresh_memory();
    system.total_memory()
}

/// The decision part of the estimate.
///
/// Kept separate from the probing so it can be exercised with synthetic memory
/// figures: the answer must not depend on how much RAM the test machine has.
fn judge(
    model_size_bytes: u64,
    kv_cache_bytes: u64,
    gpu_layers: u32,
    available: u64,
) -> (u64, CheckLevel, Vec<ValidationIssue>, Vec<String>) {
    let need = model_size_bytes
        .saturating_add(kv_cache_bytes)
        .saturating_add(RUNTIME_OVERHEAD_BYTES);

    let mut issues = Vec::new();
    let mut notes = vec![
        "memory use is estimated from the model file size, the context window, and a fixed overhead"
            .to_string(),
    ];

    let level = if model_size_bytes == 0 {
        issues.push(ValidationIssue::blocked(
            "model_path",
            "select a model file before estimating memory",
        ));
        CheckLevel::Blocked
    } else if available == 0 {
        notes.push("available memory could not be read on this system".to_string());
        CheckLevel::Warning
    } else if need > available {
        issues.push(ValidationIssue::blocked(
            "context_size",
            format!(
                "about {} of memory is needed but only {} is available; reduce the context size or the GPU layers",
                format_bytes(need),
                format_bytes(available)
            ),
        ));
        CheckLevel::Blocked
    } else if (need as f64) * HEADROOM_WARNING_RATIO > available as f64 {
        issues.push(ValidationIssue::warning(
            "context_size",
            format!(
                "about {} of memory is needed and {} is available; the system may swap, so consider a smaller context",
                format_bytes(need),
                format_bytes(available)
            ),
        ));
        CheckLevel::Warning
    } else {
        CheckLevel::Ok
    };

    if gpu_layers > 0 {
        notes.push(
            "GPU memory is not estimated: usage depends on the GPU, the driver, and the llama.cpp backend"
                .to_string(),
        );
    } else {
        notes.push(
            "the configuration runs on the CPU, so all weights and the KV cache use system memory"
                .to_string(),
        );
    }

    (need, level, issues, notes)
}

/// Estimates what the configuration will need and compares it with free memory.
pub fn estimate_resources(config: &LocalAiConfig, model_size_bytes: u64) -> ResourceEstimate {
    let available = available_memory_bytes();
    let total = total_memory_bytes();
    let kv_cache = KV_BYTES_PER_TOKEN_ESTIMATE.saturating_mul(config.server.context_size as u64);
    let (need, level, issues, notes) = judge(
        model_size_bytes,
        kv_cache,
        config.server.gpu_layers,
        available,
    );
    // The comparison is widened so that an absurd configuration cannot wrap.
    let headroom =
        (available as i128 - need as i128).clamp(i64::MIN as i128, i64::MAX as i128) as i64;

    ResourceEstimate {
        model_size_bytes,
        context_size: config.server.context_size,
        kv_cache_bytes: kv_cache,
        overhead_bytes: RUNTIME_OVERHEAD_BYTES,
        estimated_need_bytes: need,
        available_ram_bytes: available,
        total_ram_bytes: total,
        headroom_bytes: headroom,
        level,
        gpu_layers_requested: config.server.gpu_layers,
        notes,
        issues,
    }
}

/// Formats a byte count for display, without a locale dependency.
pub fn format_bytes(bytes: u64) -> String {
    const KIB: f64 = 1024.0;
    const MIB: f64 = KIB * 1024.0;
    const GIB: f64 = MIB * 1024.0;
    let value = bytes as f64;
    if value >= GIB {
        format!("{:.1} GiB", value / GIB)
    } else if value >= MIB {
        format!("{:.0} MiB", value / MIB)
    } else if value >= KIB {
        format!("{:.0} KiB", value / KIB)
    } else {
        format!("{bytes} B")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::local::config::LocalModelConfig;

    fn config_with(context_size: u32, gpu_layers: u32) -> LocalAiConfig {
        LocalAiConfig {
            server: LocalModelConfig {
                context_size,
                gpu_layers,
                ..LocalModelConfig::default()
            },
            ..LocalAiConfig::default()
        }
    }

    #[test]
    fn the_estimate_grows_with_the_context_window() {
        let small = estimate_resources(&config_with(2048, 0), 5 * 1024 * 1024 * 1024);
        let large = estimate_resources(&config_with(32768, 0), 5 * 1024 * 1024 * 1024);
        assert!(large.kv_cache_bytes > small.kv_cache_bytes);
        assert!(large.estimated_need_bytes > small.estimated_need_bytes);
        assert_eq!(
            small.estimated_need_bytes,
            5 * 1024 * 1024 * 1024 + small.kv_cache_bytes + RUNTIME_OVERHEAD_BYTES
        );
    }

    #[test]
    fn a_missing_model_blocks_the_estimate() {
        let estimate = estimate_resources(&config_with(8192, 0), 0);
        assert!(estimate.is_blocked());
        assert!(estimate
            .issues
            .iter()
            .any(|issue| issue.field == "model_path"));
    }

    #[test]
    fn an_impossible_model_size_is_blocked_and_a_tight_fit_warns() {
        const GIB: u64 = 1024 * 1024 * 1024;
        let kv = KV_BYTES_PER_TOKEN_ESTIMATE * 1024;

        // Larger than any realistic machine: blocked.
        let huge = estimate_resources(&config_with(8192, 0), u64::MAX / 2);
        assert!(huge.is_blocked());
        assert!(huge.headroom_bytes < 0);

        // On a synthetic 16 GiB machine the three bands are checked directly, so
        // the outcome does not depend on this machine's free memory.
        const AVAILABLE: u64 = 16 * GIB;
        let base = RUNTIME_OVERHEAD_BYTES + kv;

        // Needs more than is available: blocked.
        let (need, level, issues, _) = judge(AVAILABLE, kv, 0, AVAILABLE);
        assert!(need > AVAILABLE);
        assert_eq!(level, CheckLevel::Blocked);
        assert!(issues.iter().any(|issue| issue.field == "context_size"));

        // Fits, but leaves under a quarter of the memory free: warning.
        let tight_model = AVAILABLE - base - AVAILABLE / 8;
        let (need, level, issues, _) = judge(tight_model, kv, 0, AVAILABLE);
        assert!(need <= AVAILABLE);
        assert_eq!(level, CheckLevel::Warning);
        assert!(issues.iter().any(|issue| issue.field == "context_size"));

        // Fits with room to spare: no issue at all.
        let (need, level, issues, notes) = judge(AVAILABLE / 4, kv, 0, AVAILABLE);
        assert!(need * 2 <= AVAILABLE);
        assert_eq!(level, CheckLevel::Ok);
        assert!(issues.is_empty());
        assert!(notes.iter().any(|note| note.contains("CPU")));

        // A saturated model size must not wrap the arithmetic.
        let (need, level, _, _) = judge(u64::MAX, u64::MAX, 0, AVAILABLE);
        assert_eq!(need, u64::MAX);
        assert_eq!(level, CheckLevel::Blocked);

        // Half of the available memory: fine.
        let comfortable = estimate_resources(&config_with(2048, 0), 64 * 1024 * 1024);
        assert_eq!(comfortable.level, CheckLevel::Ok);
        assert!(comfortable.headroom_bytes > 0);
    }

    #[test]
    fn unreadable_memory_warns_instead_of_blocking_or_panicking() {
        let (need, level, issues, notes) = judge(5 * 1024 * 1024 * 1024, 1024, 0, 0);
        assert_eq!(level, CheckLevel::Warning);
        assert!(issues.is_empty());
        assert!(need > 0);
        assert!(notes.iter().any(|note| note.contains("could not be read")));
    }

    #[test]
    fn gpu_offload_is_reported_as_an_unestimated_heuristic() {
        let with_gpu = estimate_resources(&config_with(8192, 24), 5 * 1024 * 1024 * 1024);
        assert_eq!(with_gpu.gpu_layers_requested, 24);
        assert!(with_gpu
            .notes
            .iter()
            .any(|note| note.contains("GPU memory is not estimated")));

        let cpu_only = estimate_resources(&config_with(8192, 0), 5 * 1024 * 1024 * 1024);
        assert!(cpu_only.notes.iter().any(|note| note.contains("CPU")));
        assert!(cpu_only
            .notes
            .iter()
            .all(|note| !note.contains("telemetry")));
    }

    #[test]
    fn byte_formatting_is_readable() {
        assert_eq!(format_bytes(512), "512 B");
        assert_eq!(format_bytes(2048), "2 KiB");
        assert_eq!(format_bytes(5 * 1024 * 1024), "5 MiB");
        assert_eq!(format_bytes(3 * 1024 * 1024 * 1024), "3.0 GiB");
    }

    #[test]
    fn memory_probing_returns_something_on_a_real_machine() {
        // On a machine without readable memory statistics this may be zero; the
        // estimate then reports a warning instead of blocking.
        let total = total_memory_bytes();
        assert!(total == 0 || total > 256 * 1024 * 1024);
    }
}
