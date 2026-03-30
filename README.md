# humanize_input

Windows-native human-like typing simulator built with C# and WPF.

中文文档: [README.zh-CN.md](README.zh-CN.md)

## Features Implemented

- Character-by-character typing from pasted text
- Typing speed and jitter controls
- Typo/omission/transposition simulation
- Neighbor-key typo model (distance-decayed probability on keyboard layout)
- Repair rate control (not all mistakes are corrected)
- Delayed correction behavior (sometimes types 1-2 chars before fixing)
- Digits are excluded from typo simulation
- Hybrid injection path: Unicode for CJK + keyboard path for common ASCII keys
- Global hotkeys for Start and Pause/Resume (customizable)
- Auto-minimize to tray on startup, double-click tray icon to restore
- Auto-wait on focus loss and continue when focus returns
- Proportional UI scaling (sliders remain visible in windowed mode)
- Automatic settings persistence to INI
- Auto-load settings on startup, auto-save after option changes
- First launch creates default `settings.ini`

## Project Structure

- src/HumanizeInput.App: WPF UI and ViewModel
- src/HumanizeInput.Core: Typing session and randomization engine
- src/HumanizeInput.Infra: Windows SendInput implementation
- tests/HumanizeInput.Core.Tests: Core unit tests

## Prerequisites

Install .NET 8 SDK and Windows Desktop Runtime first.

## Build and Run

```powershell
dotnet build .\humanize_input.sln
dotnet run --project .\src\HumanizeInput.App\HumanizeInput.App.csproj
```

## Usage

1. Paste target text into the main textbox.
2. Adjust typing parameters (speed, jitter, typo rate, omission rate, transposition rate, repair rate).
3. Configure and apply global hotkeys (default: Start `Ctrl+Alt+S`, Pause/Resume `Ctrl+Alt+P`).
4. The app auto-minimizes to tray. Double-click tray icon to restore.
5. Focus the target editor input area, then press Start hotkey.
6. Press Pause hotkey anytime to pause/resume.

## Settings File (INI)

- Default location: same folder as the executable (`settings.ini`).
- On first launch, default settings are written automatically.
- After users change options, settings are auto-saved.
- On next launch, settings are auto-loaded from the INI file.

## Recommended Human-like Settings

- Base delay: 70-140 ms
- Jitter: 15%-35%
- Typo rate: 5%-10%
- Omission rate: 2%-6%
- Transposition rate: 2%-5%
- Repair rate: 70%-90%

## Notes

- Follows Windows input model: typing is sent to the foreground focused window.
- If target app runs as Administrator, run this app at the same privilege level.
- If build fails due to locked DLLs, close running humanize_input process first, then rebuild.
