//! Feature-owned oracle state and local transitions. Network and signing effects stay in the coordinator.
use super::{
    oracle_forms::OracleFormDraft,
    oracle_model::{
        OracleAction, OracleIntroAction, OracleSubmissionRecord, SpreadOracleLiveState,
        SpreadOracleRewardState,
    },
};
use crate::oracle_tui::{DEFAULT_ORACLE_NODE_INDEX, OracleIndexTree};
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) enum OracleView {
    Earn,
    #[default]
    Advanced,
}

pub(super) const ORACLE_LOCK_FLASH_TICKS: u8 = 6;
pub(super) const ORACLE_FORM_FIELD_FLASH_TICKS: u8 = 6;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct OracleLockedFlash {
    pub(super) action: OracleAction,
    pub(super) ticks_remaining: u8,
    pub(super) visible: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct OracleFormFieldFlash {
    pub(super) field_index: usize,
    pub(super) ticks_remaining: u8,
    pub(super) visible: bool,
}

pub(super) struct OracleState {
    pub(super) intro_selected: usize,
    pub(super) view: OracleView,
    pub(super) earn_selected: usize,
    pub(super) selected: usize,
    pub(super) node_selected: usize,
    pub(super) search_input: String,
    pub(super) search_editing: bool,
    pub(super) form: Option<OracleFormDraft>,
    pub(super) form_field_flash: Option<OracleFormFieldFlash>,
    pub(super) locked_flash: Option<OracleLockedFlash>,
    pub(super) tree: Option<OracleIndexTree>,
    pub(super) tree_issue: Option<String>,
    pub(super) tree_retry_after_tick: Option<usize>,
    pub(super) submissions: Vec<OracleSubmissionRecord>,
    pub(super) submission_store_path: Option<PathBuf>,
    pub(super) submission_issue: Option<String>,
    pub(super) live: Option<SpreadOracleLiveState>,
    pub(super) live_issue: Option<String>,
    pub(super) rewards: Option<SpreadOracleRewardState>,
    pub(super) reward_issue: Option<String>,
    pub(super) tree_request: u64,
    pub(super) live_request: u64,
    pub(super) reward_request: u64,
    pub(super) loading_tree: bool,
    pub(super) loading_live: bool,
    pub(super) loading_rewards: bool,
}

impl OracleState {
    pub(super) fn tick_flashes(&mut self) {
        if let Some(flash) = self.form_field_flash.as_mut() {
            if flash.ticks_remaining == 0 {
                self.form_field_flash = None;
            } else {
                flash.visible = !flash.visible;
                flash.ticks_remaining = flash.ticks_remaining.saturating_sub(1);
                if flash.ticks_remaining == 0 {
                    self.form_field_flash = None;
                }
            }
        }
        if let Some(flash) = self.locked_flash.as_mut() {
            if flash.ticks_remaining == 0 {
                self.locked_flash = None;
            } else {
                flash.visible = !flash.visible;
                flash.ticks_remaining = flash.ticks_remaining.saturating_sub(1);
                if flash.ticks_remaining == 0 {
                    self.locked_flash = None;
                }
            }
        }
    }
    pub(super) fn new(
        submission_store_path: Option<PathBuf>,
        submissions: Vec<OracleSubmissionRecord>,
        submission_issue: Option<String>,
    ) -> Self {
        Self {
            intro_selected: 0,
            view: OracleView::Advanced,
            earn_selected: 0,
            selected: 0,
            node_selected: DEFAULT_ORACLE_NODE_INDEX,
            search_input: String::new(),
            search_editing: false,
            form: None,
            form_field_flash: None,
            locked_flash: None,
            tree: None,
            tree_issue: None,
            tree_retry_after_tick: None,
            submissions,
            submission_store_path,
            submission_issue,
            live: None,
            live_issue: None,
            rewards: None,
            reward_issue: None,
            tree_request: 0,
            live_request: 0,
            reward_request: 0,
            loading_tree: false,
            loading_live: false,
            loading_rewards: false,
        }
    }

    pub(super) fn selected_intro_action(&self) -> OracleIntroAction {
        OracleIntroAction::ALL
            .get(self.intro_selected)
            .copied()
            .unwrap_or(OracleIntroAction::Earn)
    }

    pub(super) fn select_prev_intro_action(&mut self) -> bool {
        if self.intro_selected == 0 {
            return false;
        }
        self.intro_selected -= 1;
        true
    }

    pub(super) fn select_next_intro_action(&mut self) -> bool {
        if self.intro_selected + 1 >= OracleIntroAction::ALL.len() {
            return false;
        }
        self.intro_selected += 1;
        true
    }

    pub(super) fn tree_load_status(&self) -> String {
        if self.loading_tree {
            "Oracle source recipe is still loading.".to_string()
        } else if self.tree_issue.is_some() {
            "Could not load oracle source recipe.".to_string()
        } else {
            "Oracle source recipe is not loaded yet.".to_string()
        }
    }

    pub(super) fn flash_form_field(&mut self, field_index: usize) {
        self.form_field_flash = Some(OracleFormFieldFlash {
            field_index,
            ticks_remaining: ORACLE_FORM_FIELD_FLASH_TICKS,
            visible: true,
        });
    }

    pub(super) fn form_field_flash_visible(&self, field_index: usize) -> bool {
        self.form_field_flash
            .filter(|flash| flash.field_index == field_index)
            .map(|flash| flash.visible)
            .unwrap_or(false)
    }

    pub(super) fn flash_action_locked(&mut self, action: OracleAction) {
        self.locked_flash = Some(OracleLockedFlash {
            action,
            ticks_remaining: ORACLE_LOCK_FLASH_TICKS,
            visible: false,
        });
    }

    pub(super) fn action_flash_visible(&self, action: OracleAction) -> bool {
        self.locked_flash
            .filter(|flash| flash.action == action)
            .map(|flash| flash.visible)
            .unwrap_or(false)
    }
}
