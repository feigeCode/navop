use gpui::actions;

actions!(
    terminal_view,
    [
        SendTab,
        SendShiftTab,
        Copy,
        Paste,
        SelectAll,
        ClearSelection,
        ClearScreen,
        SearchForward,
        SearchBackward,
        ToggleViMode,
        ViModeStartSelection,
        IncreaseFont,
        DecreaseFont,
        ResetFont,
        StartRecording,
        PauseRecording,
        ResumeRecording,
        StopRecording,
    ]
);
