//! Exhaustive governance classification for this program's first-byte ABI.
//!
//! The instruction registry remains authoritative: classification calls its actual `from_byte`
//! decoder. Every byte classifies as exactly one of the variants below, and only the two
//! mutating classes ever reach the governance envelope or a handler.

use crate::instruction::OracleInstructionTag;

/// Bytes assigned to oracle instructions that this build does not implement yet. They reject
/// exactly like an unassigned byte, and no other instruction may claim them.
pub const RESERVED_INSTRUCTION_TAGS: [u8; 1] = [205];

/// Oracle v2, heads on pikes (`docs/heads.md`): routed only by builds with the `heads` feature
/// (a default feature, so the production artifact routes it; the dev profiles and the chain enable
/// it too). A `--no-default-features` build classifies it as unassigned, so it rejects before any
/// account is read.
pub const HEADS_INSTRUCTION_TAG: u8 = 210;

/// Bytes of the localnet dev bootstrap (`localnet-dev-authority`, `localnet-governance`) and its
/// month reset (245). Every other build classifies them as unassigned.
pub const DEV_INSTRUCTION_TAGS: [u8; 6] = [240, 241, 242, 243, 244, 245];

/// Bytes routed only by an explicitly selected feature build: the heads family and the dev
/// bootstrap.
pub const FEATURE_GATED_INSTRUCTION_TAGS: [u8; 7] = [
    HEADS_INSTRUCTION_TAG,
    DEV_INSTRUCTION_TAGS[0],
    DEV_INSTRUCTION_TAGS[1],
    DEV_INSTRUCTION_TAGS[2],
    DEV_INSTRUCTION_TAGS[3],
    DEV_INSTRUCTION_TAGS[4],
    DEV_INSTRUCTION_TAGS[5],
];

/// The v1 monthly formation (feature `v1-formation`): instructions that only form a v1 month,
/// routed only by builds that form months (the dev profiles, the test-sbf build, the chain). The
/// production artifact is a lean oracle booted at a new address: it never has a v1 month
/// (quarters go through heads, 210/28), so it classifies these bytes as unassigned and refuses
/// them before any account is read. Groups: month creation and its economics (1, 110, 181, 190);
/// the reward vault and budgets (154-158); proposals and coverage (182, 183, 184, 186, 196);
/// source challenges (161, 185, 192); openings and the move to Game (80, 81, 162-165); recipe
/// weights and the recipe index (84, 86, 187, 200); active weights (121, 122, 124); the CFM launch
/// (130); CloseOracleMonth (59; 30/24 closes a month itself). The address's own setup (0, 90, 128,
/// then 129) stays out of this set: a fresh address needs it, and heads reads neither 110's
/// economics nor 154's vault (`tests/production_fresh_boot.rs`). Ascending.
pub const V1_FORMATION_INSTRUCTION_TAGS: [u8; 32] = [
    1, 59, 80, 81, 84, 86, 110, 121, 122, 124, 130, 154, 155, 156, 157, 158, 161, 162, 163, 164,
    165, 181, 182, 183, 184, 185, 186, 187, 190, 192, 196, 200,
];

/// Tag 30 selectors of the v1 formation, routed only with `v1-formation`: carry formation (0-7),
/// the median certificates (8-13) and the evidence backfills (16, 17). Without the feature they
/// reject like an unassigned selector. Selector 22's Activate (an underwritten source's proposal)
/// rejects the same way; the rest of selector 22 is `V1_MONTH_CARRY_SELECTORS`. Ascending.
pub const V1_FORMATION_CARRY_SELECTORS: [u8; 16] =
    [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 16, 17];

/// The v1 months' machinery after formation (feature `v1-months`, which `v1-formation` implies):
/// everything a v1 month needs from Game to its last refund. The production artifact boots at an
/// address that has no v1 month, so no account any of these reads can exist there: it refuses
/// these bytes as unassigned, before any account is read. The dev profiles, the test-sbf build
/// and the chain keep them. Groups: settlement (75, 113, 116, 197) and the rotation of its signer
/// set (91-94; only 116 reads the set, heads settles without signers, 109 and 129 read it as it
/// stands); Game updates (166, 168, 198, 199, 201); the v1 escrow and bond refunds (167, 169, 191,
/// 194, 195: each holds a bond a v1 month took); reward registration and claims (170, 171, 173,
/// 174). Every way the heads oracle's money leaves stays in every build: collateral (9, 10, 11,
/// 140), the evidence writer (30/14, 30/15) and heads (210); so do the address's setup and admin
/// (0, 2, 90, 109, 128, 129) and FailPeriod (79, which also fails a heads period that never
/// lists). Ascending.
pub const V1_MONTH_INSTRUCTION_TAGS: [u8; 22] = [
    75, 91, 92, 93, 94, 113, 116, 166, 167, 168, 169, 170, 171, 173, 174, 191, 194, 195, 197, 198,
    199, 201,
];

/// Tag 30 selectors of the v1 months' machinery, routed only with `v1-months`: the council (18,
/// with the CFM launch receipt check; heads reads the controller's council without it),
/// sponsorship settlement (22; Activate is formation-only), the terminal cleanup (24) and the
/// failed-month closes (25). Without the feature they reject like an unassigned selector.
/// Ascending.
pub const V1_MONTH_CARRY_SELECTORS: [u8; 4] = [18, 22, 24, 25];

/// Which feature-gated families a build routes.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct FeatureGates {
    /// The localnet dev bootstrap (240-245).
    pub dev: bool,
    /// The heads family (210).
    pub heads: bool,
    /// The v1 monthly formation (`V1_FORMATION_INSTRUCTION_TAGS`).
    pub v1_formation: bool,
    /// The v1 months' machinery after formation (`V1_MONTH_INSTRUCTION_TAGS`); every build with
    /// `v1_formation` has it.
    pub v1_months: bool,
}

impl FeatureGates {
    pub const NONE: Self = Self {
        dev: false,
        heads: false,
        v1_formation: false,
        v1_months: false,
    };
    pub const ALL: Self = Self {
        dev: true,
        heads: true,
        v1_formation: true,
        v1_months: true,
    };

    /// The families the currently compiled build routes.
    pub const fn active() -> Self {
        Self {
            dev: cfg!(any(
                feature = "localnet-dev-authority",
                feature = "localnet-governance"
            )),
            heads: cfg!(feature = "heads"),
            v1_formation: cfg!(feature = "v1-formation"),
            v1_months: cfg!(feature = "v1-months"),
        }
    }

    /// Whether `tag` is a feature-gated byte this build routes. Inlined so a build's constant
    /// gates fold away (a build keeps no trace of the gated bytes it does not route).
    #[inline(always)]
    pub fn routes(self, tag: u8) -> bool {
        (self.heads && tag == HEADS_INSTRUCTION_TAG)
            || (self.dev && DEV_INSTRUCTION_TAGS.contains(&tag))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InstructionGovernanceClass {
    Unknown,
    Reserved,
    RecognizedReadOnly,
    RecognizedMutating,
    FeatureGatedMutating,
}

impl InstructionGovernanceClass {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Unknown => "Unknown",
            Self::Reserved => "Reserved",
            Self::RecognizedReadOnly => "RecognizedReadOnly",
            Self::RecognizedMutating => "RecognizedMutating",
            Self::FeatureGatedMutating => "FeatureGatedMutating",
        }
    }
}

/// Classify one byte for an explicitly selected build shape.
///
/// The ordinary `from_byte` decoder runs first because it is the executable registry.
/// Exhaustive tests separately prove that feature and reserved bytes never collide with it.
pub fn classify_instruction_tag_for_build(
    tag: u8,
    gates: FeatureGates,
) -> InstructionGovernanceClass {
    if !gates.v1_months && is_v1_month_tag(tag) {
        return InstructionGovernanceClass::Unknown;
    }
    classify_before_v1_months(tag, gates)
}

/// Every class but the v1 months' bytes (`classify_instruction_tag_for_build`). The program
/// classifies through this, checking those bytes first only in a build without `v1-months`
/// (`classify_active_instruction_tag`), so a build with it routes exactly as before the feature.
fn classify_before_v1_months(tag: u8, gates: FeatureGates) -> InstructionGovernanceClass {
    if OracleInstructionTag::from_byte(tag).is_some() {
        if !gates.v1_formation && is_v1_formation_tag(tag) {
            InstructionGovernanceClass::Unknown
        } else {
            InstructionGovernanceClass::RecognizedMutating
        }
    } else if gates.routes(tag) {
        InstructionGovernanceClass::FeatureGatedMutating
    } else if RESERVED_INSTRUCTION_TAGS.contains(&tag) {
        InstructionGovernanceClass::Reserved
    } else {
        InstructionGovernanceClass::Unknown
    }
}

const fn byte_set<const N: usize>(bytes: [u8; N]) -> [u64; 4] {
    let mut set = [0u64; 4];
    let mut i = 0;
    while i < N {
        set[(bytes[i] >> 6) as usize] |= 1 << (bytes[i] & 63);
        i += 1;
    }
    set
}

const V1_FORMATION_TAG_SET: [u64; 4] = byte_set(V1_FORMATION_INSTRUCTION_TAGS);
const V1_FORMATION_CARRY_SELECTOR_SET: [u64; 4] = byte_set(V1_FORMATION_CARRY_SELECTORS);
const V1_MONTH_TAG_SET: [u64; 4] = byte_set(V1_MONTH_INSTRUCTION_TAGS);
const V1_MONTH_CARRY_SELECTOR_SET: [u64; 4] = byte_set(V1_MONTH_CARRY_SELECTORS);

/// Whether `tag` belongs to the v1 formation (`V1_FORMATION_INSTRUCTION_TAGS`).
#[inline(always)]
pub fn is_v1_formation_tag(tag: u8) -> bool {
    V1_FORMATION_TAG_SET[usize::from(tag >> 6)] & (1 << (tag & 63)) != 0
}

/// Whether `selector` is a tag-30 selector of the v1 formation (`V1_FORMATION_CARRY_SELECTORS`).
#[inline(always)]
pub fn is_v1_formation_carry_selector(selector: u8) -> bool {
    V1_FORMATION_CARRY_SELECTOR_SET[usize::from(selector >> 6)] & (1 << (selector & 63)) != 0
}

/// Whether `tag` belongs to the v1 months' machinery (`V1_MONTH_INSTRUCTION_TAGS`).
#[inline(always)]
pub fn is_v1_month_tag(tag: u8) -> bool {
    V1_MONTH_TAG_SET[usize::from(tag >> 6)] & (1 << (tag & 63)) != 0
}

/// Whether `selector` is a tag-30 selector of the v1 months' machinery
/// (`V1_MONTH_CARRY_SELECTORS`).
#[inline(always)]
pub fn is_v1_month_carry_selector(selector: u8) -> bool {
    V1_MONTH_CARRY_SELECTOR_SET[usize::from(selector >> 6)] & (1 << (selector & 63)) != 0
}

/// Classify one byte exactly as the currently compiled feature build routes it.
pub fn classify_active_instruction_tag(tag: u8) -> InstructionGovernanceClass {
    if cfg!(feature = "localnet-governance") && tag == 243 {
        return InstructionGovernanceClass::Unknown;
    }
    #[cfg(not(feature = "v1-months"))]
    if is_v1_month_tag(tag) {
        return InstructionGovernanceClass::Unknown;
    }
    classify_before_v1_months(tag, FeatureGates::active())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_byte_has_exactly_one_class_and_registries_never_collide() {
        let mut counts = [0usize; 5];
        for tag in 0u8..=u8::MAX {
            let recognized = OracleInstructionTag::from_byte(tag).is_some();
            let reserved = RESERVED_INSTRUCTION_TAGS.contains(&tag);
            let gated = FEATURE_GATED_INSTRUCTION_TAGS.contains(&tag);
            assert!(
                usize::from(recognized) + usize::from(reserved) + usize::from(gated) <= 1,
                "byte {tag} is claimed twice"
            );
            let class = classify_active_instruction_tag(tag);
            assert_eq!(
                class,
                if cfg!(feature = "localnet-governance") && tag == 243 {
                    InstructionGovernanceClass::Unknown
                } else {
                    classify_instruction_tag_for_build(tag, FeatureGates::active())
                }
            );
            if !gated && !is_v1_formation_tag(tag) && !is_v1_month_tag(tag) {
                assert_eq!(
                    class,
                    classify_instruction_tag_for_build(tag, FeatureGates::ALL)
                );
            }
            counts[match class {
                InstructionGovernanceClass::Unknown => 0,
                InstructionGovernanceClass::Reserved => 1,
                InstructionGovernanceClass::RecognizedReadOnly => 2,
                InstructionGovernanceClass::RecognizedMutating => 3,
                InstructionGovernanceClass::FeatureGatedMutating => 4,
            }] += 1;
        }
        assert_eq!(counts[1], RESERVED_INSTRUCTION_TAGS.len());
        assert_eq!(counts[2], 0, "no read-only instruction is routed");
        let gates = FeatureGates::active();
        assert_eq!(
            counts[3],
            OracleInstructionTag::ALL.len()
                - if gates.v1_formation {
                    0
                } else {
                    V1_FORMATION_INSTRUCTION_TAGS.len()
                }
                - if gates.v1_months {
                    0
                } else {
                    V1_MONTH_INSTRUCTION_TAGS.len()
                }
        );
        assert_eq!(
            counts[4],
            if gates.dev {
                DEV_INSTRUCTION_TAGS.len() - usize::from(cfg!(feature = "localnet-governance"))
            } else {
                0
            } + usize::from(gates.heads)
        );
        assert_eq!(counts.iter().sum::<usize>(), 256);
        let mut sorted = RESERVED_INSTRUCTION_TAGS;
        sorted.sort_unstable();
        assert_eq!(
            sorted, RESERVED_INSTRUCTION_TAGS,
            "reserved bytes stay sorted"
        );
        assert!(sorted.windows(2).all(|pair| pair[0] < pair[1]));
    }

    #[test]
    fn the_heads_byte_routes_only_where_heads_is_compiled_in() {
        assert!(OracleInstructionTag::from_byte(HEADS_INSTRUCTION_TAG).is_none());
        assert!(!RESERVED_INSTRUCTION_TAGS.contains(&HEADS_INSTRUCTION_TAG));
        assert!(!DEV_INSTRUCTION_TAGS.contains(&HEADS_INSTRUCTION_TAG));
        for (gates, expected) in [
            (FeatureGates::NONE, InstructionGovernanceClass::Unknown),
            (
                FeatureGates {
                    dev: true,
                    heads: false,
                    v1_formation: true,
                    v1_months: true,
                },
                InstructionGovernanceClass::Unknown,
            ),
            (
                FeatureGates {
                    dev: false,
                    heads: true,
                    v1_formation: true,
                    v1_months: true,
                },
                InstructionGovernanceClass::FeatureGatedMutating,
            ),
            (
                FeatureGates::ALL,
                InstructionGovernanceClass::FeatureGatedMutating,
            ),
        ] {
            assert_eq!(
                classify_instruction_tag_for_build(HEADS_INSTRUCTION_TAG, gates),
                expected,
                "{gates:?}"
            );
            // The heads gate never routes a dev byte, nor the dev gate the heads byte.
            for tag in DEV_INSTRUCTION_TAGS {
                assert_eq!(
                    classify_instruction_tag_for_build(tag, gates)
                        == InstructionGovernanceClass::FeatureGatedMutating,
                    gates.dev,
                    "{gates:?} byte {tag}"
                );
            }
        }
        // The compiled build: one without `heads` (`--no-default-features`) rejects 210 as unknown.
        assert_eq!(
            classify_active_instruction_tag(HEADS_INSTRUCTION_TAG),
            if cfg!(feature = "heads") {
                InstructionGovernanceClass::FeatureGatedMutating
            } else {
                InstructionGovernanceClass::Unknown
            }
        );
        // Every dev profile compiles the heads family in.
        if cfg!(any(
            feature = "localnet-dev-authority",
            feature = "localnet-governance"
        )) {
            assert!(cfg!(feature = "heads"));
        }
    }

    #[test]
    fn v1_formation_bytes_route_only_where_v1_formation_is_compiled_in() {
        for set in [
            &V1_FORMATION_INSTRUCTION_TAGS[..],
            &V1_FORMATION_CARRY_SELECTORS[..],
        ] {
            assert!(set.windows(2).all(|pair| pair[0] < pair[1]), "ascending");
        }
        for byte in 0u8..=u8::MAX {
            assert_eq!(
                is_v1_formation_tag(byte),
                V1_FORMATION_INSTRUCTION_TAGS.contains(&byte),
                "{byte}"
            );
            assert_eq!(
                is_v1_formation_carry_selector(byte),
                V1_FORMATION_CARRY_SELECTORS.contains(&byte),
                "{byte}"
            );
        }
        for tag in V1_FORMATION_INSTRUCTION_TAGS {
            // An assigned registry byte, never a feature or reserved one.
            assert!(OracleInstructionTag::from_byte(tag).is_some(), "{tag}");
            assert!(!FEATURE_GATED_INSTRUCTION_TAGS.contains(&tag));
            for (gates, expected) in [
                (FeatureGates::NONE, InstructionGovernanceClass::Unknown),
                (
                    FeatureGates {
                        dev: true,
                        heads: true,
                        v1_formation: false,
                        v1_months: true,
                    },
                    InstructionGovernanceClass::Unknown,
                ),
                (
                    FeatureGates {
                        dev: false,
                        heads: false,
                        v1_formation: true,
                        v1_months: true,
                    },
                    InstructionGovernanceClass::RecognizedMutating,
                ),
                (
                    FeatureGates::ALL,
                    InstructionGovernanceClass::RecognizedMutating,
                ),
            ] {
                assert_eq!(
                    classify_instruction_tag_for_build(tag, gates),
                    expected,
                    "{gates:?} byte {tag}"
                );
            }
            assert_eq!(
                classify_active_instruction_tag(tag),
                if cfg!(feature = "v1-formation") {
                    InstructionGovernanceClass::RecognizedMutating
                } else {
                    InstructionGovernanceClass::Unknown
                }
            );
        }
        // Every other assigned byte outside the v1 months' machinery routes in every build; the
        // dev profiles and the test-sbf build form months.
        for tag in OracleInstructionTag::ALL {
            if !is_v1_formation_tag(tag as u8) && !is_v1_month_tag(tag as u8) {
                assert_eq!(
                    classify_instruction_tag_for_build(tag as u8, FeatureGates::NONE),
                    InstructionGovernanceClass::RecognizedMutating
                );
            }
        }
        if cfg!(any(
            feature = "localnet-dev-authority",
            feature = "localnet-governance",
            feature = "test-sbf"
        )) {
            assert!(cfg!(feature = "v1-formation"));
        }
    }

    #[test]
    fn v1_month_bytes_route_only_where_v1_months_is_compiled_in() {
        for set in [
            &V1_MONTH_INSTRUCTION_TAGS[..],
            &V1_MONTH_CARRY_SELECTORS[..],
        ] {
            assert!(set.windows(2).all(|pair| pair[0] < pair[1]), "ascending");
        }
        for byte in 0u8..=u8::MAX {
            assert_eq!(
                is_v1_month_tag(byte),
                V1_MONTH_INSTRUCTION_TAGS.contains(&byte),
                "{byte}"
            );
            assert_eq!(
                is_v1_month_carry_selector(byte),
                V1_MONTH_CARRY_SELECTORS.contains(&byte),
                "{byte}"
            );
            // The two families never share a byte or a selector.
            assert!(
                !(is_v1_month_tag(byte) && is_v1_formation_tag(byte)),
                "{byte}"
            );
            assert!(
                !(is_v1_month_carry_selector(byte) && is_v1_formation_carry_selector(byte)),
                "{byte}"
            );
        }
        let production = FeatureGates {
            heads: true,
            ..FeatureGates::NONE
        };
        let live = FeatureGates {
            v1_months: true,
            ..production
        };
        for tag in V1_MONTH_INSTRUCTION_TAGS {
            // An assigned registry byte, never a feature or reserved one.
            assert!(OracleInstructionTag::from_byte(tag).is_some(), "{tag}");
            assert!(!FEATURE_GATED_INSTRUCTION_TAGS.contains(&tag));
            for (gates, expected) in [
                (FeatureGates::NONE, InstructionGovernanceClass::Unknown),
                (production, InstructionGovernanceClass::Unknown),
                (live, InstructionGovernanceClass::RecognizedMutating),
                (
                    FeatureGates::ALL,
                    InstructionGovernanceClass::RecognizedMutating,
                ),
            ] {
                assert_eq!(
                    classify_instruction_tag_for_build(tag, gates),
                    expected,
                    "{gates:?} byte {tag}"
                );
            }
            assert_eq!(
                classify_active_instruction_tag(tag),
                if cfg!(feature = "v1-months") {
                    InstructionGovernanceClass::RecognizedMutating
                } else {
                    InstructionGovernanceClass::Unknown
                }
            );
        }
        // The build with the v1 months and no formation routes every assigned byte outside the
        // formation; the production artifact also refuses these.
        for tag in OracleInstructionTag::ALL {
            let tag = tag as u8;
            assert_eq!(
                classify_instruction_tag_for_build(tag, live),
                if is_v1_formation_tag(tag) {
                    InstructionGovernanceClass::Unknown
                } else {
                    InstructionGovernanceClass::RecognizedMutating
                },
                "{tag}"
            );
            assert_eq!(
                classify_instruction_tag_for_build(tag, production)
                    == InstructionGovernanceClass::RecognizedMutating,
                !is_v1_formation_tag(tag) && !is_v1_month_tag(tag),
                "{tag}"
            );
        }
        // Forming months implies running them.
        if cfg!(feature = "v1-formation") {
            assert!(cfg!(feature = "v1-months"));
        }
    }

    /// The bytes the production artifact routes, listed apart from the two sets on purpose:
    /// dropping one, or letting a v1 byte back in, is a failure here.
    #[test]
    fn the_production_artifact_routes_exactly_a_fresh_address_setup_and_the_heads_oracle() {
        let production = FeatureGates {
            heads: true,
            ..FeatureGates::NONE
        };
        let routed: Vec<u8> = (0u8..=u8::MAX)
            .filter(|tag| {
                matches!(
                    classify_instruction_tag_for_build(*tag, production),
                    InstructionGovernanceClass::RecognizedMutating
                        | InstructionGovernanceClass::FeatureGatedMutating
                )
            })
            .collect();
        assert_eq!(
            routed,
            [
                0,
                2,
                9,
                10,
                11,
                30,
                79,
                90,
                109,
                128,
                129,
                140,
                HEADS_INSTRUCTION_TAG
            ]
        );
        // A fresh address's setup stays out of the formation; what heads does not read stays in.
        for tag in [0, 90, 128, 129] {
            assert!(!is_v1_formation_tag(tag) && !is_v1_month_tag(tag), "{tag}");
        }
        for tag in [110, 154] {
            assert!(is_v1_formation_tag(tag), "{tag}");
        }
        for tag in [167, 169, 191, 194, 195] {
            assert!(is_v1_month_tag(tag), "{tag}");
        }
        // Tag 30 keeps the evidence writer; the terminal cleanup goes with the months.
        assert!(is_v1_month_carry_selector(24));
        for selector in [14, 15] {
            assert!(
                !is_v1_formation_carry_selector(selector) && !is_v1_month_carry_selector(selector),
                "{selector}"
            );
        }
    }

    #[test]
    fn class_names_are_stable() {
        for (class, name) in [
            (InstructionGovernanceClass::Unknown, "Unknown"),
            (InstructionGovernanceClass::Reserved, "Reserved"),
            (
                InstructionGovernanceClass::RecognizedReadOnly,
                "RecognizedReadOnly",
            ),
            (
                InstructionGovernanceClass::RecognizedMutating,
                "RecognizedMutating",
            ),
            (
                InstructionGovernanceClass::FeatureGatedMutating,
                "FeatureGatedMutating",
            ),
        ] {
            assert_eq!(class.as_str(), name);
        }
    }
}
