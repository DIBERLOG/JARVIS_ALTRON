//! Disk-space lifecycle for a managed installation, and the proof of its numbers.
//!
//! # Why this module exists
//!
//! An earlier version of this code asked for *two* copies of the model plus a
//! reserve, for every installation:
//!
//! ```text
//! 2 * 5_027_783_488 + 536_870_912 = 10_592_437_888
//! ```
//!
//! That number is wrong for a clean install, because the file operations of this
//! application never hold two copies of a *new* model. It is also incomplete,
//! because it forgets the runtime entirely. This module replaces the arithmetic
//! with a simulation of the real file layout, so the requirement can be read off
//! the operations instead of guessed.
//!
//! # The file lifecycle
//!
//! Every temporary path lives under one directory inside the managed data root,
//! on the same volume as the final location. Every promotion is therefore a
//! `rename`, never a `copy`:
//!
//! ```text
//! setup-temp/<component>/<name>.part          download, hashed in place
//! setup-temp/<component>/payload/<name>       rename of the .part (no copy)
//! <final directory>/<name>                    rename of the payload directory
//! ```
//!
//! | step                     | operation | second full copy? |
//! |--------------------------|-----------|-------------------|
//! | download `.part`         | write     | no                |
//! | SHA-256 / GGUF / PE      | read      | no                |
//! | `.part` -> `payload`     | rename    | no                |
//! | `payload` -> final       | rename    | no                |
//! | archive extraction       | copy      | the runtime only (see below) |
//!
//! The runtime archive is the one place where two copies of *different* things
//! coexist: the compressed archive and the files unpacked from it. Both are tiny
//! next to the model, are bounded by
//! [`MAX_RUNTIME_UNPACKED_BYTES`](super::managed::MAX_RUNTIME_UNPACKED_BYTES), and
//! the archive is deleted as soon as the extraction is verified.
//!
//! # The two cases
//!
//! * **Clean install** — nothing is installed yet, so the peak is one full new
//!   model plus the unpacked runtime plus the reserve.
//! * **Update** — a previous working model must survive until the new one is
//!   validated and committed, so the peak is *two* models plus the runtime plus
//!   the reserve. The previous version is only counted when it is actually
//!   installed; a plan for a machine that has no managed model is a clean
//!   install, not an update.
//!
//! Every number below is asserted by tests, and `setup::layout` proves the same
//! claim on a real filesystem by measuring the tree after every renaming step.

use serde::{Deserialize, Serialize};

use super::super::managed::{managed_model_manifest, managed_runtime_manifest};
use super::super::managed::MAX_RUNTIME_UNPACKED_BYTES;

/// Free space kept untouched so a full disk cannot make the installation fail
/// halfway. It also covers the runtime's installation receipt and the settings
/// file that activation rewrites.
pub const SAFETY_RESERVE_BYTES: u64 = 512 * 1024 * 1024;

/// Bytes the unpacked runtime is planned to occupy.
///
/// The exact figure is only known after extraction, so the planner uses the hard
/// bound the extractor enforces. Planning above the truth is safe; planning below
/// it is not, and inventing a smaller "typical" number would be a guess.
pub const RUNTIME_PLAN_BYTES: u64 = MAX_RUNTIME_UNPACKED_BYTES;

/// Whether a component has to be written, or is already usable as it is.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ComponentPlanKind {
    /// Already installed, validated, and at the pinned version.
    Keep,
    /// Absent, partial, damaged, or at another version.
    Install,
}

/// Which of the three disk cases applies to the model.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelInstallKind {
    /// A validated managed model is already installed: nothing is written.
    Keep,
    /// No managed model is installed: one full copy is ever on disk.
    Clean,
    /// A working managed model stays until the new one is committed: two copies.
    Update,
}

/// A per-component summary of what the plan will write and retain.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ComponentPlan {
    pub kind: ComponentPlanKind,
    /// Bytes this component adds to the disk.
    pub new_bytes: u64,
    /// Bytes of a previously installed version that must stay while it does.
    pub retained_bytes: u64,
}

/// Peak-and-current ledger for the simulated file layout.
///
/// `write` counts bytes that are in the temporary tree *and* on the disk;
/// `commit` moves them out of the temporary tree without freeing them; `retain`
/// counts bytes that were already on the disk and may not be freed; `release`
/// frees bytes.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct DiskLedger {
    live: u64,
    peak: u64,
    temporary: u64,
    temporary_peak: u64,
}

impl DiskLedger {
    /// Writes `bytes` into the temporary tree.
    pub fn write(&mut self, bytes: u64) {
        self.live = self.live.saturating_add(bytes);
        self.temporary = self.temporary.saturating_add(bytes);
        self.peak = self.peak.max(self.live);
        self.temporary_peak = self.temporary_peak.max(self.temporary);
    }

    /// Keeps `bytes` that are already on the disk and must not be freed yet.
    pub fn retain(&mut self, bytes: u64) {
        self.live = self.live.saturating_add(bytes);
        self.peak = self.peak.max(self.live);
    }

    /// Frees `bytes` from both the disk and the temporary tree.
    pub fn release(&mut self, bytes: u64) {
        self.live = self.live.saturating_sub(bytes);
        self.temporary = self.temporary.saturating_sub(bytes);
    }

    /// Promotes `bytes` out of the temporary tree. They stay on the disk.
    pub fn commit(&mut self, bytes: u64) {
        self.temporary = self.temporary.saturating_sub(bytes);
    }

    /// Highest disk usage the plan reaches.
    pub fn peak(&self) -> u64 {
        self.peak
    }

    /// Highest usage of the temporary tree the plan reaches.
    pub fn temporary_peak(&self) -> u64 {
        self.temporary_peak
    }

    /// Disk usage once the plan has finished.
    pub fn live(&self) -> u64 {
        self.live
    }
}

/// Everything the arithmetic depends on, with no probing inside.
///
/// Keeping the probe out of the plan is what lets a test assert the numbers on
/// any machine, including one with nearly no free space.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PlanInputs {
    /// Size of the pinned model file.
    pub model_bytes: u64,
    /// Size of the pinned runtime archive.
    pub runtime_archive_bytes: u64,
    /// Planned footprint of the unpacked runtime.
    pub runtime_unpacked_bytes: u64,
    pub runtime: ComponentPlanKind,
    pub model: ModelInstallKind,
    /// Bytes of an installed runtime at another version that stays on the disk.
    pub runtime_retained_bytes: u64,
    /// Bytes of the installed model that must survive until the commit.
    pub model_retained_bytes: u64,
    /// Free space on the volume that holds the managed root.
    pub available_bytes: u64,
}

/// The plan, in the terms the interface reports.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SpacePlan {
    pub model_bytes: u64,
    pub runtime_bytes: u64,
    /// Bytes to fetch over the network, for the components that need fetching.
    pub download_bytes: u64,
    /// Highest usage of the temporary tree.
    pub temporary_bytes: u64,
    /// Bytes the plan adds to the installed tree.
    pub installed_bytes: u64,
    /// Bytes of a previous version kept for rollback.
    pub rollback_bytes: u64,
    pub safety_reserve_bytes: u64,
    /// Free space the installation needs at its worst moment.
    pub required_peak_bytes: u64,
    pub available_bytes: u64,
    /// Free space still missing; zero when the plan fits.
    pub missing_bytes: u64,
    pub runtime: ComponentPlan,
    pub model: ComponentPlan,
}

impl SpacePlan {
    /// Whether the machine has enough free space.
    pub fn fits(&self) -> bool {
        self.missing_bytes == 0
    }

    /// Whether a previous version is being kept, so the interface can say so.
    pub fn keeps_previous_version(&self) -> bool {
        self.rollback_bytes > 0
    }
}

/// Simulates the file lifecycle and returns the resulting space plan.
///
/// The sequence is the one `setup::coordinator` performs, in that order:
///
/// 1. runtime archive `.part` (temporary), then the unpacked staging copy, then
///    the archive is deleted, then the staging directory is renamed into place;
/// 2. any runtime kept from another version stays;
/// 3. an installed model is retained when the model is an update;
/// 4. the new model `.part` is renamed into staging, then into the model
///    directory.
pub fn plan_space(inputs: PlanInputs) -> SpacePlan {
    let mut ledger = DiskLedger::default();

    if inputs.runtime == ComponentPlanKind::Install {
        ledger.write(inputs.runtime_archive_bytes);
        ledger.write(inputs.runtime_unpacked_bytes);
        // The archive is deleted after the extraction is verified.
        ledger.release(inputs.runtime_archive_bytes);
        // The verified staging directory becomes the installed runtime.
        ledger.commit(inputs.runtime_unpacked_bytes);
    }
    ledger.retain(inputs.runtime_retained_bytes);

    let mut rollback_bytes = inputs.runtime_retained_bytes;
    match inputs.model {
        ModelInstallKind::Keep => {}
        ModelInstallKind::Clean => {
            ledger.write(inputs.model_bytes);
            ledger.commit(inputs.model_bytes);
        }
        ModelInstallKind::Update => {
            // The previous model is on the disk already; the plan must not free
            // it before the new one has been validated and committed.
            ledger.retain(inputs.model_retained_bytes);
            rollback_bytes = rollback_bytes.saturating_add(inputs.model_retained_bytes);
            ledger.write(inputs.model_bytes);
            ledger.commit(inputs.model_bytes);
        }
    }

    let runtime_new = if inputs.runtime == ComponentPlanKind::Install {
        inputs.runtime_unpacked_bytes
    } else {
        0
    };
    let model_new = if inputs.model == ModelInstallKind::Keep {
        0
    } else {
        inputs.model_bytes
    };
    let download_bytes = if inputs.runtime == ComponentPlanKind::Install {
        inputs.runtime_archive_bytes.saturating_add(model_new)
    } else {
        model_new
    };
    let installed_bytes = runtime_new.saturating_add(model_new);
    let required_peak_bytes = ledger.peak().saturating_add(SAFETY_RESERVE_BYTES);

    SpacePlan {
        model_bytes: inputs.model_bytes,
        runtime_bytes: inputs.runtime_unpacked_bytes,
        download_bytes,
        temporary_bytes: ledger.temporary_peak(),
        installed_bytes,
        rollback_bytes,
        safety_reserve_bytes: SAFETY_RESERVE_BYTES,
        required_peak_bytes,
        available_bytes: inputs.available_bytes,
        missing_bytes: required_peak_bytes.saturating_sub(inputs.available_bytes),
        runtime: ComponentPlan {
            kind: inputs.runtime,
            new_bytes: runtime_new,
            retained_bytes: inputs.runtime_retained_bytes,
        },
        // The model is written in both writing cases; only the retention
        // differs, and `ModelInstallKind` already carries that distinction. A
        // `Clean` plan retains nothing even if a caller passed a retention.
        model: ComponentPlan {
            kind: match inputs.model {
                ModelInstallKind::Keep => ComponentPlanKind::Keep,
                _ => ComponentPlanKind::Install,
            },
            new_bytes: model_new,
            retained_bytes: match inputs.model {
                ModelInstallKind::Update => inputs.model_retained_bytes,
                _ => 0,
            },
        },
    }
}

/// The plan for a machine with no managed runtime and no managed model.
///
/// This is the number the setup page shows before anything is downloaded.
pub fn clean_install_plan(available_bytes: u64) -> SpacePlan {
    let runtime = managed_runtime_manifest();
    let model = managed_model_manifest();
    plan_space(PlanInputs {
        model_bytes: model.artifact.expected_size,
        runtime_archive_bytes: runtime.artifact.expected_size,
        runtime_unpacked_bytes: RUNTIME_PLAN_BYTES,
        runtime: ComponentPlanKind::Install,
        model: ModelInstallKind::Clean,
        runtime_retained_bytes: 0,
        model_retained_bytes: 0,
        available_bytes,
    })
}

/// The plan for replacing an installed model while keeping it until the commit.
pub fn update_plan(available_bytes: u64) -> SpacePlan {
    let runtime = managed_runtime_manifest();
    let model = managed_model_manifest();
    plan_space(PlanInputs {
        model_bytes: model.artifact.expected_size,
        runtime_archive_bytes: runtime.artifact.expected_size,
        runtime_unpacked_bytes: RUNTIME_PLAN_BYTES,
        runtime: ComponentPlanKind::Install,
        model: ModelInstallKind::Update,
        runtime_retained_bytes: 0,
        model_retained_bytes: model.artifact.expected_size,
        available_bytes,
    })
}

/// Free space a clean installation of the pinned artifacts requires, without
/// reference to any particular machine.
pub fn clean_install_required_bytes() -> u64 {
    clean_install_plan(u64::MAX).required_peak_bytes
}

/// Free space an update of the pinned model requires, with the previous version
/// retained until the commit.
pub fn update_required_bytes() -> u64 {
    update_plan(u64::MAX).required_peak_bytes
}

#[cfg(test)]
mod tests {
    use super::*;

    const MODEL: u64 = 5_027_783_488;
    const ARCHIVE: u64 = 18_427_629;
    const MIB: u64 = 1024 * 1024;

    fn pinned() -> (u64, u64) {
        (
            managed_model_manifest().artifact.expected_size,
            managed_runtime_manifest().artifact.expected_size,
        )
    }

    #[test]
    fn the_manifests_still_hold_the_pinned_sizes() {
        let (model, archive) = pinned();
        assert_eq!(model, MODEL);
        assert_eq!(archive, ARCHIVE);
        // The decimal and binary readings of the model file, as the interface
        // shows them: 4.68 GiB, and about 5.03 GB as a vendor would print it.
        let gib = MODEL as f64 / (1024.0 * 1024.0 * 1024.0);
        let gb = MODEL as f64 / 1_000_000_000.0;
        assert!((gib - 4.68).abs() < 0.01, "4.68 GiB, got {gib}");
        assert!((gb - 5.03).abs() < 0.01, "about 5.03 GB, got {gb}");
    }

    #[test]
    fn a_clean_install_holds_exactly_one_full_model() {
        let plan = clean_install_plan(u64::MAX);
        assert_eq!(plan.model.kind, ComponentPlanKind::Install);
        assert_eq!(plan.model.retained_bytes, 0);
        assert_eq!(plan.rollback_bytes, 0);
        assert_eq!(plan.installed_bytes, RUNTIME_PLAN_BYTES + MODEL);
        assert_eq!(plan.download_bytes, ARCHIVE + MODEL);
        // The four numbers the report quotes.
        assert_eq!(plan.model_bytes, MODEL);
        assert_eq!(plan.runtime_bytes, RUNTIME_PLAN_BYTES);
        assert_eq!(plan.safety_reserve_bytes, 512 * MIB);
        assert_eq!(
            plan.required_peak_bytes,
            RUNTIME_PLAN_BYTES + MODEL + 512 * MIB
        );
        assert_eq!(plan.required_peak_bytes, 5_665_317_696);
        assert!(plan.fits());
        // The peak is one model plus a small runtime, never two models.
        assert!(plan.required_peak_bytes < 2 * MODEL);
        // The temporary tree holds one model at its worst: the `.part` file is
        // the only full copy that ever exists in it, and the runtime's staging
        // directory is much smaller than the model.
        assert_eq!(plan.temporary_bytes, MODEL);
    }

    #[test]
    fn an_update_holds_a_second_copy_only_because_it_is_kept() {
        let plan = update_plan(u64::MAX);
        assert_eq!(plan.rollback_bytes, MODEL);
        assert_eq!(plan.model.retained_bytes, MODEL);
        assert_eq!(plan.installed_bytes, RUNTIME_PLAN_BYTES + MODEL);
        assert_eq!(
            plan.required_peak_bytes,
            RUNTIME_PLAN_BYTES + 2 * MODEL + 512 * MIB
        );
        assert_eq!(plan.required_peak_bytes, 10_693_101_184);
        assert!(plan.keeps_previous_version());
        // The update is the only case that needs more than one model.
        assert!(plan.required_peak_bytes > clean_install_required_bytes());
    }

    #[test]
    fn the_shortcuts_agree_with_the_full_plans() {
        assert_eq!(clean_install_required_bytes(), clean_install_plan(0).required_peak_bytes);
        assert_eq!(update_required_bytes(), update_plan(0).required_peak_bytes);
        assert_eq!(clean_install_required_bytes(), 5_665_317_696);
        assert_eq!(update_required_bytes(), 10_693_101_184);
        // The old formula over-charged a clean install and forgot the runtime.
        assert_ne!(clean_install_required_bytes(), 10_592_437_888);
        assert_ne!(update_required_bytes(), 10_592_437_888);
    }

    #[test]
    fn the_runtime_participates_and_a_kept_runtime_costs_nothing() {
        let model = managed_model_manifest();
        let base = PlanInputs {
            model_bytes: model.artifact.expected_size,
            runtime_archive_bytes: ARCHIVE,
            runtime_unpacked_bytes: RUNTIME_PLAN_BYTES,
            runtime: ComponentPlanKind::Install,
            model: ModelInstallKind::Clean,
            runtime_retained_bytes: 0,
            model_retained_bytes: 0,
            available_bytes: u64::MAX,
        };
        let fresh = plan_space(base);
        let kept = plan_space(PlanInputs {
            runtime: ComponentPlanKind::Keep,
            ..base
        });
        assert_eq!(fresh.runtime.new_bytes, RUNTIME_PLAN_BYTES);
        assert_eq!(kept.runtime.new_bytes, 0);
        assert_eq!(kept.runtime.kind, ComponentPlanKind::Keep);
        // No archive and no unpacked copy are written for a runtime that is kept.
        assert_eq!(kept.download_bytes, MODEL);
        assert_eq!(kept.installed_bytes, MODEL);
        assert_eq!(kept.required_peak_bytes, MODEL + 512 * MIB);
        assert!(kept.required_peak_bytes < fresh.required_peak_bytes);
    }

    #[test]
    fn missing_space_is_reported_instead_of_clamped_away() {
        let plan = clean_install_plan(1_000_000_000);
        assert!(!plan.fits());
        assert_eq!(plan.available_bytes, 1_000_000_000);
        assert_eq!(
            plan.missing_bytes,
            clean_install_required_bytes() - 1_000_000_000
        );
        // Exactly enough fits; one byte less does not.
        let exact = clean_install_plan(clean_install_required_bytes());
        assert!(exact.fits());
        assert_eq!(exact.missing_bytes, 0);
        let one_short = clean_install_plan(clean_install_required_bytes() - 1);
        assert!(!one_short.fits());
        assert_eq!(one_short.missing_bytes, 1);
    }

    #[test]
    fn a_retained_runtime_from_another_version_is_counted() {
        let model = managed_model_manifest();
        let plan = plan_space(PlanInputs {
            model_bytes: model.artifact.expected_size,
            runtime_archive_bytes: ARCHIVE,
            runtime_unpacked_bytes: RUNTIME_PLAN_BYTES,
            runtime: ComponentPlanKind::Install,
            model: ModelInstallKind::Clean,
            runtime_retained_bytes: 40 * MIB,
            model_retained_bytes: 0,
            available_bytes: u64::MAX,
        });
        assert_eq!(plan.rollback_bytes, 40 * MIB);
        assert_eq!(plan.runtime.retained_bytes, 40 * MIB);
        assert_eq!(
            plan.required_peak_bytes,
            RUNTIME_PLAN_BYTES + 40 * MIB + MODEL + 512 * MIB
        );
    }

    #[test]
    fn a_machine_that_already_has_everything_needs_only_the_reserve() {
        let plan = plan_space(PlanInputs {
            model_bytes: MODEL,
            runtime_archive_bytes: ARCHIVE,
            runtime_unpacked_bytes: RUNTIME_PLAN_BYTES,
            runtime: ComponentPlanKind::Keep,
            model: ModelInstallKind::Keep,
            runtime_retained_bytes: 0,
            model_retained_bytes: 0,
            available_bytes: u64::MAX,
        });
        assert_eq!(plan.download_bytes, 0);
        assert_eq!(plan.installed_bytes, 0);
        assert_eq!(plan.temporary_bytes, 0);
        assert_eq!(plan.rollback_bytes, 0);
        assert_eq!(plan.model.kind, ComponentPlanKind::Keep);
        assert_eq!(plan.required_peak_bytes, 512 * MIB);
    }

    #[test]
    fn the_ledger_tracks_a_peak_and_never_wraps() {
        let mut ledger = DiskLedger::default();
        ledger.write(10);
        assert_eq!(ledger.live(), 10);
        assert_eq!(ledger.temporary_peak(), 10);
        ledger.write(5);
        ledger.release(5);
        ledger.commit(10);
        assert_eq!(ledger.temporary_peak(), 15);
        assert_eq!(ledger.live(), 10);
        // Saturating on purpose: a broken input must not produce a small number.
        let mut saturated = DiskLedger::default();
        saturated.write(u64::MAX);
        saturated.write(u64::MAX);
        assert_eq!(saturated.peak(), u64::MAX);
        saturated.release(u64::MAX);
        saturated.release(u64::MAX);
        assert_eq!(saturated.live(), 0);
    }

    #[test]
    fn the_runtime_plan_bound_is_the_extractor_bound() {
        // Planning below what the extractor can write would under-charge the
        // installation, so the two numbers are the same constant.
        assert_eq!(RUNTIME_PLAN_BYTES, MAX_RUNTIME_UNPACKED_BYTES);
        assert_eq!(RUNTIME_PLAN_BYTES, 96 * MIB);
    }
}
