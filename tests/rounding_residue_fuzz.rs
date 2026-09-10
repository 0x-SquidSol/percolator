//! ADL effective-quantity round-trip checks from upstream
//! tests/rounding_residue_fuzz.rs (6f36972d). Only the effective-quantity
//! tests are here; the rest of upstream's file covers mechanisms this fork
//! has not adopted yet. Built with `--features fuzz` like tests/v16_fuzzing.rs.
#![cfg(feature = "fuzz")]

use percolator::{
    kani_adl_effective_quantity_ceil, kani_raw_basis_for_adl_effective_quantity, ADL_ONE,
    MAX_POSITION_ABS_Q, MIN_A_SIDE, POS_SCALE,
};
use proptest::prelude::*;

#[test]
fn adl_effective_quantity_roundtrip_boundary_partition() {
    let raw_values = [
        0,
        1,
        POS_SCALE - 1,
        POS_SCALE,
        POS_SCALE + 1,
        MAX_POSITION_ABS_Q - 1,
        MAX_POSITION_ABS_Q,
    ];
    let a_basis_values = [
        MIN_A_SIDE,
        MIN_A_SIDE + 1,
        ADL_ONE / 3,
        ADL_ONE / 2,
        ADL_ONE - 1,
        ADL_ONE,
    ];
    let current_a_values = [
        1,
        MIN_A_SIDE - 1,
        MIN_A_SIDE,
        MIN_A_SIDE + 1,
        ADL_ONE / 3,
        ADL_ONE / 2,
        ADL_ONE - 1,
        ADL_ONE,
    ];
    for raw_abs_q in raw_values {
        for a_basis in a_basis_values {
            for current_a in current_a_values.into_iter().filter(|a| *a <= a_basis) {
                let effective =
                    kani_adl_effective_quantity_ceil(raw_abs_q, a_basis, current_a).unwrap();
                let targets = [0, effective / 2, effective.saturating_sub(1)];
                for target_effective in targets {
                    if effective == 0 || target_effective >= effective {
                        continue;
                    }
                    let target_raw = kani_raw_basis_for_adl_effective_quantity(
                        target_effective,
                        a_basis,
                        current_a,
                    )
                    .unwrap();
                    assert!(target_raw <= raw_abs_q);
                    assert_eq!(
                        kani_adl_effective_quantity_ceil(target_raw, a_basis, current_a),
                        Ok(target_effective),
                    );
                }
            }
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(20000))]

    #[test]
    fn adl_effective_quantity_roundtrip_preserves_any_reachable_reduction(
        raw_abs_q in 0u128..=MAX_POSITION_ABS_Q,
        a_basis in MIN_A_SIDE..=ADL_ONE,
        current_selector in any::<u128>(),
        target_selector in any::<u128>(),
    ) {
        let current_a = 1 + current_selector % a_basis;
        let current_effective =
            kani_adl_effective_quantity_ceil(raw_abs_q, a_basis, current_a).unwrap();
        let target_effective = if current_effective == 0 {
            0
        } else {
            target_selector % current_effective
        };
        let target_raw = kani_raw_basis_for_adl_effective_quantity(
            target_effective,
            a_basis,
            current_a,
        )
        .unwrap();

        prop_assert!(target_raw <= raw_abs_q);
        prop_assert_eq!(
            kani_adl_effective_quantity_ceil(target_raw, a_basis, current_a),
            Ok(target_effective),
        );
    }
}
