//! Interactive Petri terminal facade.
//!
//! Runtime behavior lives in the private `lab::app` composition tree. This
//! module intentionally exposes only the stable crate-facing launch and exit
//! contract used by command dispatch.

mod app;

pub use app::{
    LabExitAction, TUI_REBUILD_REQUESTED_EXIT_CODE, TUI_UPDATE_REQUESTED_EXIT_CODE, run_lab_bench,
    run_lab_chart_bench,
};
