using System.ComponentModel;
using System.Runtime.CompilerServices;
using System.Windows;
using System.Windows.Input;
using System.Windows.Threading;
using HumanizeInput.App.Analysis;
using HumanizeInput.App.Commands;
using HumanizeInput.App.Settings;
using HumanizeInput.Core;
using HumanizeInput.Core.Models;

namespace HumanizeInput.App.ViewModels;

public sealed class MainViewModel : INotifyPropertyChanged
{
    private const string LanguageZhCn = "zh-CN";
    private const string LanguageEnUs = "en-US";

    private readonly TypingSessionService _session;
    private readonly ITypingDriver _driver;
    private readonly RelayCommand _startCommand;
    private readonly RelayCommand _pauseResumeCommand;
    private readonly RelayCommand _applyHotkeysCommand;
    private readonly RelayCommand _toggleLanguageCommand;
    private readonly IniSettingsStore _settingsStore;
    private readonly DispatcherTimer _saveDebounceTimer;
    private bool _isApplyingLoadedSettings;

    private string _inputText = string.Empty;
    private string _currentLanguage = LanguageEnUs;
    private int _baseDelayMs = 90;
    private int _jitterPercent = 20;
    private int _typoRatePercent = 8;
    private int _omissionRatePercent = 5;
    private int _transposeRatePercent = 4;
    private int _repairRatePercent = 85;
    private int _errorDetectDelayMs = 900;
    private int _backspaceDelayMs = 70;
    private int _leadInDelayMs = 2500;
    private string _statusText = string.Empty;
    private string _progressText = string.Empty;
    private string _statusDetail = string.Empty;
    private TypingProgress _lastProgress = new();
    private string _startHotkeyText = "Ctrl+Alt+S";
    private string _pauseHotkeyText = "Ctrl+Alt+P";

    public MainViewModel(TypingSessionService session, ITypingDriver driver, IniSettingsStore settingsStore)
    {
        _session = session;
        _driver = driver;
        _settingsStore = settingsStore;

        _startCommand = new RelayCommand(StartSession, () => !_session.IsBusy);
        _pauseResumeCommand = new RelayCommand(TogglePauseResume, () => _session.IsBusy);
        _applyHotkeysCommand = new RelayCommand(ApplyHotkeys);
        _toggleLanguageCommand = new RelayCommand(ToggleLanguage);

        _session.StateChanged += OnStateChanged;
        _session.ProgressChanged += OnProgressChanged;
        _session.LogProduced += OnLogProduced;

        _saveDebounceTimer = new DispatcherTimer
        {
            Interval = TimeSpan.FromMilliseconds(600)
        };
        _saveDebounceTimer.Tick += (_, _) => SaveSettingsNow();

        LoadSettings();
        SetStatusDetail(LocalizeState(SessionState.Idle));
        UpdateProgressText();
    }

    public event PropertyChangedEventHandler? PropertyChanged;

    public ICommand StartCommand => _startCommand;
    public ICommand PauseResumeCommand => _pauseResumeCommand;
    public ICommand ApplyHotkeysCommand => _applyHotkeysCommand;
    public ICommand ToggleLanguageCommand => _toggleLanguageCommand;

    public event Action<string, string>? HotkeyUpdateRequested;
    public event Action? UiLanguageChanged;

    public string UiWindowTitle => "humanize_input";
    public string InputSectionTitle => Translate("要输入的文本", "Text To Type");
    public string TypingRhythmSectionTitle => Translate("输入节奏", "Typing Rhythm");
    public string BaseDelayLabelText => Translate("基础延迟 (ms)", "Base Delay (ms)");
    public string BaseDelayDisplayText => Translate($"当前: {BaseDelayMs} ms", $"Current: {BaseDelayMs} ms");
    public string JitterLabelText => Translate("抖动百分比 (%)", "Jitter Percent (%)");
    public string JitterDisplayText => Translate($"当前: {JitterPercent}%", $"Current: {JitterPercent}%");
    public string LeadInLabelText => Translate("开始前准备时间 (ms)", "Lead-in Delay (ms)");
    public string LeadInDisplayText => Translate($"当前: {LeadInDelayMs} ms", $"Current: {LeadInDelayMs} ms");
    public string ErrorSimulationSectionTitle => Translate("错误模拟", "Error Simulation");
    public string TypoRateLabelText => Translate("错字率 (%)", "Typo Rate (%)");
    public string TypoRateDisplayText => Translate($"当前: {TypoRatePercent}%", $"Current: {TypoRatePercent}%");
    public string OmissionRateLabelText => Translate("漏写率 (%)", "Omission Rate (%)");
    public string OmissionRateDisplayText => Translate($"当前: {OmissionRatePercent}%", $"Current: {OmissionRatePercent}%");
    public string TransposeRateLabelText => Translate("颠倒率 (%)", "Transposition Rate (%)");
    public string TransposeRateDisplayText => Translate($"当前: {TransposeRatePercent}%", $"Current: {TransposeRatePercent}%");
    public string RepairRateLabelText => Translate("修复率 (%)", "Repair Rate (%)");
    public string RepairRateDisplayText => Translate($"当前: {RepairRatePercent}%", $"Current: {RepairRatePercent}%");
    public string ErrorDetectDelayLabelText => Translate("发现错误后停顿 (ms)", "Error Detect Delay (ms)");
    public string ErrorDetectDelayDisplayText => Translate($"当前: {ErrorDetectDelayMs} ms", $"Current: {ErrorDetectDelayMs} ms");
    public string BackspaceDelayLabelText => Translate("回删间隔 (ms)", "Backspace Delay (ms)");
    public string BackspaceDelayDisplayText => Translate($"当前: {BackspaceDelayMs} ms", $"Current: {BackspaceDelayMs} ms");
    public string StartHotkeyLabelText => Translate("开始热键", "Start Hotkey");
    public string PauseHotkeyLabelText => Translate("暂停/继续热键", "Pause/Resume Hotkey");
    public string ApplyHotkeysButtonText => Translate("应用热键", "Apply Hotkeys");
    public string HotkeyExampleTooltipText => Translate("示例: Ctrl+Alt+S", "Example: Ctrl+Alt+S");
    public string TrayHintText => Translate("程序启动后会自动最小化到托盘。双击托盘图标可恢复窗口。", "The app auto-minimizes to tray on startup. Double-click the tray icon to restore.");
    public string SettingsButtonText => Translate("设置", "Settings");
    public string SettingsMenuTitleText => Translate("设置", "Settings");
    public string LanguageLabelText => Translate("语言", "Language");
    public string TypingDetectorButtonText => Translate("打开输入检测器", "Open Typing Detector");
    public string AboutSectionTitleText => Translate("关于", "About");
    public string AppNameText => "humanize_input";
    public string AppDescriptionText => Translate("Windows 原生逐字输入模拟工具", "Windows-native human-like typing simulator");
    public string AppTechStackText => Translate("C# + WPF / .NET 8", "C# + WPF / .NET 8");
    public string GitHubProfileLabelText => Translate("GitHub 主页", "GitHub Profile");
    public string GitHubRepositoryLabelText => Translate("GitHub 仓库", "GitHub Repository");
    public string OpenGitHubProfileButtonText => Translate("打开主页", "Open Profile");
    public string OpenRepositoryButtonText => Translate("打开仓库", "Open Repository");
    public string GitHubProfileValueText => "Eric-1029";
    public string GitHubRepositoryValueText => "Eric-1029/humanize_input";
    public string GitHubProfileUrl => "https://github.com/Eric-1029";
    public string GitHubRepositoryUrl => "https://github.com/Eric-1029/humanize_input";
    public string CurrentLanguageName => IsChinese ? "中文" : "English";
    public string LanguageToggleButtonText => "文 / A";
    public string TrayOpenMenuText => Translate("打开主窗口", "Open Window");
    public string TrayExitMenuText => Translate("退出", "Exit");

    public string InputText
    {
        get => _inputText;
        set
        {
            if (SetField(ref _inputText, value))
            {
                QueueSettingsSave();
            }
        }
    }

    public string CurrentLanguage
    {
        get => _currentLanguage;
        set
        {
            string normalized = NormalizeLanguage(value);
            if (SetField(ref _currentLanguage, normalized))
            {
                RaiseLocalizationChanged();
                UiLanguageChanged?.Invoke();

                if (!_isApplyingLoadedSettings)
                {
                    QueueSettingsSave();
                    SetStatusDetail(Translate("已切换语言。", "Language switched."));
                }
            }
        }
    }

    public int BaseDelayMs
    {
        get => _baseDelayMs;
        set
        {
            if (SetField(ref _baseDelayMs, value))
            {
                OnPropertyChanged(nameof(BaseDelayDisplayText));
                QueueSettingsSave();
            }
        }
    }

    public int JitterPercent
    {
        get => _jitterPercent;
        set
        {
            if (SetField(ref _jitterPercent, value))
            {
                OnPropertyChanged(nameof(JitterDisplayText));
                QueueSettingsSave();
            }
        }
    }

    public int TypoRatePercent
    {
        get => _typoRatePercent;
        set
        {
            if (SetField(ref _typoRatePercent, value))
            {
                OnPropertyChanged(nameof(TypoRateDisplayText));
                QueueSettingsSave();
            }
        }
    }

    public int OmissionRatePercent
    {
        get => _omissionRatePercent;
        set
        {
            if (SetField(ref _omissionRatePercent, value))
            {
                OnPropertyChanged(nameof(OmissionRateDisplayText));
                QueueSettingsSave();
            }
        }
    }

    public int TransposeRatePercent
    {
        get => _transposeRatePercent;
        set
        {
            if (SetField(ref _transposeRatePercent, value))
            {
                OnPropertyChanged(nameof(TransposeRateDisplayText));
                QueueSettingsSave();
            }
        }
    }

    public int RepairRatePercent
    {
        get => _repairRatePercent;
        set
        {
            if (SetField(ref _repairRatePercent, value))
            {
                OnPropertyChanged(nameof(RepairRateDisplayText));
                QueueSettingsSave();
            }
        }
    }

    public int ErrorDetectDelayMs
    {
        get => _errorDetectDelayMs;
        set
        {
            if (SetField(ref _errorDetectDelayMs, value))
            {
                OnPropertyChanged(nameof(ErrorDetectDelayDisplayText));
                QueueSettingsSave();
            }
        }
    }

    public int BackspaceDelayMs
    {
        get => _backspaceDelayMs;
        set
        {
            if (SetField(ref _backspaceDelayMs, value))
            {
                OnPropertyChanged(nameof(BackspaceDelayDisplayText));
                QueueSettingsSave();
            }
        }
    }

    public int LeadInDelayMs
    {
        get => _leadInDelayMs;
        set
        {
            if (SetField(ref _leadInDelayMs, value))
            {
                OnPropertyChanged(nameof(LeadInDisplayText));
                QueueSettingsSave();
            }
        }
    }

    public string StatusText
    {
        get => _statusText;
        set => SetField(ref _statusText, value);
    }

    public string ProgressText
    {
        get => _progressText;
        set => SetField(ref _progressText, value);
    }

    public string StartHotkeyText
    {
        get => _startHotkeyText;
        set
        {
            if (SetField(ref _startHotkeyText, value))
            {
                QueueSettingsSave();
            }
        }
    }

    public string PauseHotkeyText
    {
        get => _pauseHotkeyText;
        set
        {
            if (SetField(ref _pauseHotkeyText, value))
            {
                QueueSettingsSave();
            }
        }
    }

    public void FlushSettings()
    {
        SaveSettingsNow();
    }

    public void ReportHotkeyRegistrationResult(bool success, string startHotkeyText, string pauseHotkeyText)
    {
        if (success)
        {
            SetStatusDetail(Translate($"全局热键已生效（开始: {startHotkeyText}，暂停: {pauseHotkeyText}）", $"Global hotkeys active (Start: {startHotkeyText}, Pause: {pauseHotkeyText})."));
        }
        else
        {
            SetStatusDetail(Translate("热键注册失败，请检查格式或避免与系统热键冲突", "Hotkey registration failed. Check format or avoid conflicts with system shortcuts."));
        }
    }

    public void TriggerStartHotkey(nint targetWindow)
    {
        StartSession(targetWindow);
    }

    public void TriggerPauseResumeHotkey()
    {
        TogglePauseResume();
    }

    private void StartSession()
    {
        StartSession(targetWindowOverride: null);
    }

    private async void StartSession(nint? targetWindowOverride)
    {
        if (string.IsNullOrWhiteSpace(InputText))
        {
            System.Windows.MessageBox.Show(Translate("请先粘贴文本。", "Please paste text first."), UiWindowTitle, System.Windows.MessageBoxButton.OK, System.Windows.MessageBoxImage.Information);
            return;
        }

        SetStatusDetail(Translate($"准备开始，{LeadInDelayMs}ms 后抓取目标窗口", $"Preparing to start; capture target window in {LeadInDelayMs} ms."));
        await Task.Delay(LeadInDelayMs);

        nint targetWindow = targetWindowOverride ?? _driver.GetForegroundWindowHandle();
        TypingSettings settings = BuildSettings();

        bool started = _session.Start(InputText, settings, targetWindow);
        if (!started)
        {
            SetStatusDetail(Translate("无法开始（已有会话或输入为空）", "Unable to start (session already running or input is empty)."));
        }

        RefreshCommands();
    }

    private void TogglePauseResume()
    {
        if (_session.State == SessionState.Running)
        {
            _session.Pause();
        }
        else if (_session.State == SessionState.Paused)
        {
            _session.Resume();
        }

        RefreshCommands();
    }

    private void ApplyHotkeys()
    {
        HotkeyUpdateRequested?.Invoke(StartHotkeyText, PauseHotkeyText);
        SetStatusDetail(Translate("已请求更新全局热键", "Requested global hotkey update."));
    }

    private void ToggleLanguage()
    {
        CurrentLanguage = IsChinese ? LanguageEnUs : LanguageZhCn;
    }

    public void ApplyTypingFitResult(TypingFitResult result)
    {
        ArgumentNullException.ThrowIfNull(result);

        _isApplyingLoadedSettings = true;
        try
        {
            BaseDelayMs = result.BaseDelayMs;
            JitterPercent = result.JitterPercent;
            TypoRatePercent = result.TypoRatePercent;
            OmissionRatePercent = result.OmissionRatePercent;
            TransposeRatePercent = result.TransposeRatePercent;
            RepairRatePercent = result.RepairRatePercent;
            ErrorDetectDelayMs = result.ErrorDetectDelayMs;
            BackspaceDelayMs = result.BackspaceDelayMs;
            LeadInDelayMs = result.LeadInDelayMs;
        }
        finally
        {
            _isApplyingLoadedSettings = false;
        }

        SaveSettingsNow();
        SetStatusDetail(Translate($"已应用拟合结果，准确率 {result.AccuracyPercent:F1}%", $"Applied fitted result with {result.AccuracyPercent:F1}% accuracy."));
    }

    private TypingSettings BuildSettings()
    {
        return new TypingSettings
        {
            BaseDelayMs = BaseDelayMs,
            JitterPercent = JitterPercent,
            TypoRatePercent = TypoRatePercent,
            OmissionRatePercent = OmissionRatePercent,
            TransposeRatePercent = TransposeRatePercent,
            RepairRatePercent = RepairRatePercent,
            ErrorDetectDelayMs = ErrorDetectDelayMs,
            BackspaceDelayMs = BackspaceDelayMs,
            LeadInDelayMs = LeadInDelayMs
        };
    }

    private void LoadSettings()
    {
        _isApplyingLoadedSettings = true;
        try
        {
            UserSettings loaded = _settingsStore.LoadOrCreateDefault(BuildCurrentUserSettings());
            CurrentLanguage = loaded.Language;
            BaseDelayMs = loaded.BaseDelayMs;
            JitterPercent = loaded.JitterPercent;
            TypoRatePercent = loaded.TypoRatePercent;
            OmissionRatePercent = loaded.OmissionRatePercent;
            TransposeRatePercent = loaded.TransposeRatePercent;
            RepairRatePercent = loaded.RepairRatePercent;
            ErrorDetectDelayMs = loaded.ErrorDetectDelayMs;
            BackspaceDelayMs = loaded.BackspaceDelayMs;
            LeadInDelayMs = loaded.LeadInDelayMs;
            StartHotkeyText = loaded.StartHotkeyText;
            PauseHotkeyText = loaded.PauseHotkeyText;
        }
        finally
        {
            _isApplyingLoadedSettings = false;
        }
    }

    private UserSettings BuildCurrentUserSettings()
    {
        return new UserSettings
        {
            Language = CurrentLanguage,
            BaseDelayMs = BaseDelayMs,
            JitterPercent = JitterPercent,
            TypoRatePercent = TypoRatePercent,
            OmissionRatePercent = OmissionRatePercent,
            TransposeRatePercent = TransposeRatePercent,
            RepairRatePercent = RepairRatePercent,
            ErrorDetectDelayMs = ErrorDetectDelayMs,
            BackspaceDelayMs = BackspaceDelayMs,
            LeadInDelayMs = LeadInDelayMs,
            StartHotkeyText = StartHotkeyText,
            PauseHotkeyText = PauseHotkeyText
        };
    }

    private void QueueSettingsSave()
    {
        if (_isApplyingLoadedSettings)
        {
            return;
        }

        _saveDebounceTimer.Stop();
        _saveDebounceTimer.Start();
    }

    private void SaveSettingsNow()
    {
        _saveDebounceTimer.Stop();
        _settingsStore.Save(BuildCurrentUserSettings());
    }

    private void OnStateChanged(SessionState state)
    {
        System.Windows.Application.Current.Dispatcher.Invoke(() =>
        {
            SetStatusDetail(LocalizeState(state));
            RefreshCommands();
        });
    }

    private void OnProgressChanged(TypingProgress progress)
    {
        System.Windows.Application.Current.Dispatcher.Invoke(() =>
        {
            _lastProgress = progress;
            UpdateProgressText();
        });
    }

    private void OnLogProduced(string message)
    {
        System.Windows.Application.Current.Dispatcher.Invoke(() =>
        {
            SetStatusDetail(LocalizeLogMessage(message));
        });
    }

    private void RefreshCommands()
    {
        _startCommand.RaiseCanExecuteChanged();
        _pauseResumeCommand.RaiseCanExecuteChanged();
    }

    private void SetStatusDetail(string detail)
    {
        _statusDetail = detail;
        UpdateStatusText();
    }

    private void UpdateStatusText()
    {
        StatusText = IsChinese ? $"状态: {_statusDetail}" : $"Status: {_statusDetail}";
    }

    private void UpdateProgressText()
    {
        if (IsChinese)
        {
            ProgressText = $"进度: {_lastProgress.TypedChars}/{_lastProgress.TotalChars} | 错字: {_lastProgress.TypoCount} | 漏写: {_lastProgress.OmissionCount} | 颠倒: {_lastProgress.TransposeCount} | 修正: {_lastProgress.CorrectionCount}";
            return;
        }

        ProgressText = $"Progress: {_lastProgress.TypedChars}/{_lastProgress.TotalChars} | Typo: {_lastProgress.TypoCount} | Omission: {_lastProgress.OmissionCount} | Transpose: {_lastProgress.TransposeCount} | Repaired: {_lastProgress.CorrectionCount}";
    }

    private string LocalizeState(SessionState state)
    {
        return state switch
        {
            SessionState.Idle => Translate("空闲", "Idle"),
            SessionState.Running => Translate("输入中", "Running"),
            SessionState.Paused => Translate("已暂停", "Paused"),
            SessionState.Completed => Translate("已完成", "Completed"),
            SessionState.Faulted => Translate("错误", "Faulted"),
            _ => state.ToString()
        };
    }

    private string LocalizeLogMessage(string message)
    {
        if (IsChinese)
        {
            return message;
        }

        if (message.StartsWith("输入失败:", StringComparison.Ordinal))
        {
            string tail = message["输入失败:".Length..].Trim();
            return $"Typing failed: {tail}";
        }

        return message switch
        {
            "输入文本为空，已忽略开始请求。" => "Input text is empty; start request ignored.",
            "未检测到有效目标窗口，已取消开始。请先聚焦目标输入框再按开始热键。" => "No valid target window detected. Focus the target input first, then press Start hotkey.",
            "已有会话在运行或暂停中。" => "A session is already running or paused.",
            "会话已暂停。" => "Session paused.",
            "会话已继续。" => "Session resumed.",
            "会话已停止。" => "Session stopped.",
            "检测到目标窗口失焦，等待焦点恢复后自动继续。" => "Target window lost focus; waiting and will resume automatically when focus returns.",
            "目标窗口焦点已恢复，继续输入。" => "Target focus restored; continue typing.",
            "会话输入完成。" => "Typing session completed.",
            _ => message
        };
    }

    private void RaiseLocalizationChanged()
    {
        OnPropertyChanged(nameof(UiWindowTitle));
        OnPropertyChanged(nameof(InputSectionTitle));
        OnPropertyChanged(nameof(TypingRhythmSectionTitle));
        OnPropertyChanged(nameof(BaseDelayLabelText));
        OnPropertyChanged(nameof(BaseDelayDisplayText));
        OnPropertyChanged(nameof(JitterLabelText));
        OnPropertyChanged(nameof(JitterDisplayText));
        OnPropertyChanged(nameof(LeadInLabelText));
        OnPropertyChanged(nameof(LeadInDisplayText));
        OnPropertyChanged(nameof(ErrorSimulationSectionTitle));
        OnPropertyChanged(nameof(TypoRateLabelText));
        OnPropertyChanged(nameof(TypoRateDisplayText));
        OnPropertyChanged(nameof(OmissionRateLabelText));
        OnPropertyChanged(nameof(OmissionRateDisplayText));
        OnPropertyChanged(nameof(TransposeRateLabelText));
        OnPropertyChanged(nameof(TransposeRateDisplayText));
        OnPropertyChanged(nameof(RepairRateLabelText));
        OnPropertyChanged(nameof(RepairRateDisplayText));
        OnPropertyChanged(nameof(ErrorDetectDelayLabelText));
        OnPropertyChanged(nameof(ErrorDetectDelayDisplayText));
        OnPropertyChanged(nameof(BackspaceDelayLabelText));
        OnPropertyChanged(nameof(BackspaceDelayDisplayText));
        OnPropertyChanged(nameof(StartHotkeyLabelText));
        OnPropertyChanged(nameof(PauseHotkeyLabelText));
        OnPropertyChanged(nameof(ApplyHotkeysButtonText));
        OnPropertyChanged(nameof(HotkeyExampleTooltipText));
        OnPropertyChanged(nameof(TrayHintText));
        OnPropertyChanged(nameof(SettingsButtonText));
        OnPropertyChanged(nameof(SettingsMenuTitleText));
        OnPropertyChanged(nameof(LanguageLabelText));
        OnPropertyChanged(nameof(TypingDetectorButtonText));
        OnPropertyChanged(nameof(AboutSectionTitleText));
        OnPropertyChanged(nameof(AppNameText));
        OnPropertyChanged(nameof(AppDescriptionText));
        OnPropertyChanged(nameof(AppTechStackText));
        OnPropertyChanged(nameof(GitHubProfileLabelText));
        OnPropertyChanged(nameof(GitHubRepositoryLabelText));
        OnPropertyChanged(nameof(OpenGitHubProfileButtonText));
        OnPropertyChanged(nameof(OpenRepositoryButtonText));
        OnPropertyChanged(nameof(GitHubProfileValueText));
        OnPropertyChanged(nameof(GitHubRepositoryValueText));
        OnPropertyChanged(nameof(GitHubProfileUrl));
        OnPropertyChanged(nameof(GitHubRepositoryUrl));
        OnPropertyChanged(nameof(CurrentLanguageName));
        OnPropertyChanged(nameof(LanguageToggleButtonText));
        OnPropertyChanged(nameof(TrayOpenMenuText));
        OnPropertyChanged(nameof(TrayExitMenuText));

        UpdateStatusText();
        UpdateProgressText();
    }

    private string Translate(string zh, string en)
    {
        return IsChinese ? zh : en;
    }

    private bool IsChinese => string.Equals(_currentLanguage, LanguageZhCn, StringComparison.OrdinalIgnoreCase);

    private static string NormalizeLanguage(string? language)
    {
        if (string.IsNullOrWhiteSpace(language))
        {
            return LanguageZhCn;
        }

        return language.StartsWith("en", StringComparison.OrdinalIgnoreCase)
            ? LanguageEnUs
            : LanguageZhCn;
    }

    private void OnPropertyChanged(string propertyName)
    {
        PropertyChanged?.Invoke(this, new PropertyChangedEventArgs(propertyName));
    }

    private bool SetField<T>(ref T field, T value, [CallerMemberName] string? propertyName = null)
    {
        if (EqualityComparer<T>.Default.Equals(field, value))
        {
            return false;
        }

        field = value;
        OnPropertyChanged(propertyName ?? string.Empty);
        return true;
    }
}
