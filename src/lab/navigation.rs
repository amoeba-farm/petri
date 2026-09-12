//! Screen focus defaults, focus graph traversal, and plain focus-independent navigation helpers.

use super::*;

pub(super) fn default_focus_for_screen(screen: LabScreen) -> LabFocus {
    match screen {
        LabScreen::Terms => LabFocus::Terms,
        LabScreen::Home => LabFocus::HomeActions,
        LabScreen::Staking => LabFocus::Staking,
        LabScreen::Chain => LabFocus::Markets,
        LabScreen::Chart => LabFocus::Chart,
        LabScreen::OracleIntro => LabFocus::OracleIntro,
        LabScreen::Oracle => LabFocus::OracleTasks,
        LabScreen::OracleHelp => LabFocus::OracleHelp,
        LabScreen::Help => LabFocus::Help,
        LabScreen::Detail => LabFocus::Detail,
        LabScreen::Activity => LabFocus::Activity,
        LabScreen::Ledger => LabFocus::Ledger,
    }
}

pub(super) const FOCUS_EDGES: &[FocusEdge] = &[
    FocusEdge {
        screen: LabScreen::Home,
        from: LabFocus::Markets,
        direction: FocusDirection::Right,
        to: LabFocus::HomeSummary,
    },
    FocusEdge {
        screen: LabScreen::Home,
        from: LabFocus::MarketSeries,
        direction: FocusDirection::Right,
        to: LabFocus::HomeSummary,
    },
    FocusEdge {
        screen: LabScreen::Home,
        from: LabFocus::HomeSummary,
        direction: FocusDirection::Left,
        to: LabFocus::Markets,
    },
    FocusEdge {
        screen: LabScreen::Home,
        from: LabFocus::HomeSummary,
        direction: FocusDirection::Down,
        to: LabFocus::HomeActions,
    },
    FocusEdge {
        screen: LabScreen::Home,
        from: LabFocus::HomeActions,
        direction: FocusDirection::Left,
        to: LabFocus::Markets,
    },
    FocusEdge {
        screen: LabScreen::Home,
        from: LabFocus::HomeActions,
        direction: FocusDirection::Up,
        to: LabFocus::HomeSummary,
    },
    FocusEdge {
        screen: LabScreen::Home,
        from: LabFocus::HomePreview,
        direction: FocusDirection::Left,
        to: LabFocus::Markets,
    },
    FocusEdge {
        screen: LabScreen::Home,
        from: LabFocus::HomePreview,
        direction: FocusDirection::Up,
        to: LabFocus::HomeActions,
    },
    FocusEdge {
        screen: LabScreen::OracleIntro,
        from: LabFocus::Markets,
        direction: FocusDirection::Right,
        to: LabFocus::OracleIntro,
    },
    FocusEdge {
        screen: LabScreen::OracleIntro,
        from: LabFocus::OracleIntro,
        direction: FocusDirection::Left,
        to: LabFocus::Markets,
    },
    FocusEdge {
        screen: LabScreen::OracleHelp,
        from: LabFocus::Markets,
        direction: FocusDirection::Right,
        to: LabFocus::OracleHelp,
    },
    FocusEdge {
        screen: LabScreen::OracleHelp,
        from: LabFocus::OracleHelp,
        direction: FocusDirection::Left,
        to: LabFocus::Markets,
    },
    FocusEdge {
        screen: LabScreen::Chain,
        from: LabFocus::Markets,
        direction: FocusDirection::Right,
        to: LabFocus::Calls,
    },
    FocusEdge {
        screen: LabScreen::Chain,
        from: LabFocus::MarketSeries,
        direction: FocusDirection::Right,
        to: LabFocus::Calls,
    },
    FocusEdge {
        screen: LabScreen::Chain,
        from: LabFocus::Calls,
        direction: FocusDirection::Left,
        to: LabFocus::Markets,
    },
    FocusEdge {
        screen: LabScreen::Chain,
        from: LabFocus::Calls,
        direction: FocusDirection::Right,
        to: LabFocus::Puts,
    },
    FocusEdge {
        screen: LabScreen::Chain,
        from: LabFocus::Puts,
        direction: FocusDirection::Left,
        to: LabFocus::Calls,
    },
    FocusEdge {
        screen: LabScreen::Chart,
        from: LabFocus::Markets,
        direction: FocusDirection::Right,
        to: LabFocus::Chart,
    },
    FocusEdge {
        screen: LabScreen::Chart,
        from: LabFocus::MarketSeries,
        direction: FocusDirection::Right,
        to: LabFocus::Chart,
    },
    FocusEdge {
        screen: LabScreen::Chart,
        from: LabFocus::Chart,
        direction: FocusDirection::Left,
        to: LabFocus::Markets,
    },
    FocusEdge {
        screen: LabScreen::Oracle,
        from: LabFocus::Markets,
        direction: FocusDirection::Right,
        to: LabFocus::OracleTasks,
    },
    FocusEdge {
        screen: LabScreen::Oracle,
        from: LabFocus::MarketSeries,
        direction: FocusDirection::Right,
        to: LabFocus::OracleTasks,
    },
    FocusEdge {
        screen: LabScreen::Oracle,
        from: LabFocus::OracleTasks,
        direction: FocusDirection::Left,
        to: LabFocus::Markets,
    },
    FocusEdge {
        screen: LabScreen::Oracle,
        from: LabFocus::OracleTasks,
        direction: FocusDirection::Right,
        to: LabFocus::OracleOverview,
    },
    FocusEdge {
        screen: LabScreen::Oracle,
        from: LabFocus::OracleTasks,
        direction: FocusDirection::Down,
        to: LabFocus::OracleOverview,
    },
    FocusEdge {
        screen: LabScreen::Oracle,
        from: LabFocus::OracleOverview,
        direction: FocusDirection::Left,
        to: LabFocus::OracleTasks,
    },
    FocusEdge {
        screen: LabScreen::Oracle,
        from: LabFocus::OracleOverview,
        direction: FocusDirection::Up,
        to: LabFocus::OracleTasks,
    },
    FocusEdge {
        screen: LabScreen::Oracle,
        from: LabFocus::OracleOverview,
        direction: FocusDirection::Down,
        to: LabFocus::OracleActions,
    },
    FocusEdge {
        screen: LabScreen::Oracle,
        from: LabFocus::OracleActions,
        direction: FocusDirection::Left,
        to: LabFocus::OracleTasks,
    },
    FocusEdge {
        screen: LabScreen::Oracle,
        from: LabFocus::OracleActions,
        direction: FocusDirection::Up,
        to: LabFocus::OracleOverview,
    },
    FocusEdge {
        screen: LabScreen::Detail,
        from: LabFocus::Markets,
        direction: FocusDirection::Right,
        to: LabFocus::Detail,
    },
    FocusEdge {
        screen: LabScreen::Detail,
        from: LabFocus::MarketSeries,
        direction: FocusDirection::Right,
        to: LabFocus::Detail,
    },
    FocusEdge {
        screen: LabScreen::Detail,
        from: LabFocus::Detail,
        direction: FocusDirection::Left,
        to: LabFocus::Markets,
    },
    FocusEdge {
        screen: LabScreen::Activity,
        from: LabFocus::Markets,
        direction: FocusDirection::Right,
        to: LabFocus::Activity,
    },
    FocusEdge {
        screen: LabScreen::Activity,
        from: LabFocus::MarketSeries,
        direction: FocusDirection::Right,
        to: LabFocus::Activity,
    },
    FocusEdge {
        screen: LabScreen::Activity,
        from: LabFocus::Activity,
        direction: FocusDirection::Left,
        to: LabFocus::Markets,
    },
    FocusEdge {
        screen: LabScreen::Ledger,
        from: LabFocus::Markets,
        direction: FocusDirection::Right,
        to: LabFocus::Ledger,
    },
    FocusEdge {
        screen: LabScreen::Ledger,
        from: LabFocus::MarketSeries,
        direction: FocusDirection::Right,
        to: LabFocus::Ledger,
    },
    FocusEdge {
        screen: LabScreen::Ledger,
        from: LabFocus::Ledger,
        direction: FocusDirection::Left,
        to: LabFocus::Markets,
    },
];

pub(super) fn focus_neighbor(
    screen: LabScreen,
    from: LabFocus,
    direction: FocusDirection,
) -> Option<LabFocus> {
    FOCUS_EDGES
        .iter()
        .find(|edge| edge.screen == screen && edge.from == from && edge.direction == direction)
        .map(|edge| edge.to)
}

pub(super) fn focus_cycle_order(screen: LabScreen) -> &'static [LabFocus] {
    match screen {
        LabScreen::Terms => &[LabFocus::Terms],
        LabScreen::Staking => &[LabFocus::Staking],
        LabScreen::Home => &[
            LabFocus::Markets,
            LabFocus::HomeSummary,
            LabFocus::HomeActions,
        ],
        LabScreen::Chain => &[LabFocus::Markets, LabFocus::Calls, LabFocus::Puts],
        LabScreen::Chart => &[LabFocus::Markets, LabFocus::Chart],
        LabScreen::OracleIntro => &[LabFocus::Markets, LabFocus::OracleIntro],
        LabScreen::Oracle => &[
            LabFocus::Markets,
            LabFocus::OracleTasks,
            LabFocus::OracleActions,
            LabFocus::OracleOverview,
            LabFocus::OraclePath,
        ],
        LabScreen::OracleHelp => &[LabFocus::Markets, LabFocus::OracleHelp],
        LabScreen::Help => &[LabFocus::Help],
        LabScreen::Detail => &[LabFocus::Markets, LabFocus::Detail],
        LabScreen::Activity => &[LabFocus::Markets, LabFocus::Activity],
        LabScreen::Ledger => &[LabFocus::Markets, LabFocus::Ledger],
    }
}
