#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    // Application Lifecycle
    Quit,

    // Navigation
    SelectUp,
    SelectDown,
    SelectPageUp,
    SelectPageDown,
    SelectFirst,
    SelectLast,

    // View Toggles
    ToggleHelp,
    HelpScrollUp,
    HelpScrollDown,
    TogglePause,
    ToggleGraph,
    ToggleIndices,
    ToggleSystemIndices,
    ToggleHealth,

    // Data Operations
    ShowDetails,
    ToggleExclude,
    ClearExclusions,

    // Settings
    IncreaseRefreshRate,
    DecreaseRefreshRate,
    NextColormap,
    PrevColormap,
    NextColumn,
    PrevColumn,
    ToggleSortOrder,

    // Filter
    EnterFilterMode,
    ExitFilterMode,
    ClearFilter,

    // Details Popup
    CloseDetails,
    DetailsScrollUp,
    DetailsScrollDown,
    DetailsScrollPageUp,
    DetailsScrollPageDown,
}
