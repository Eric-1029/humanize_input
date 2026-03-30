using System.ComponentModel;
using System.Runtime.CompilerServices;
using System.Windows;
using System.Windows.Input;
using System.Windows.Threading;
using HumanizeInput.App.Commands;
using HumanizeInput.App.Settings;
using HumanizeInput.Core;
using HumanizeInput.Core.Models;

namespace HumanizeInput.App.ViewModels;

public sealed class MainViewModel : INotifyPropertyChanged
{
    private readonly TypingSessionService _session;
    private readonly ITypingDriver _driver;
    private readonly RelayCommand _startCommand;
    private readonly RelayCommand _pauseResumeCommand;
    private readonly RelayCommand _applyHotkeysCommand;
    private readonly IniSettingsStore _settingsStore;
    private readonly DispatcherTimer _saveDebounceTimer;
    private bool _isApplyingLoadedSettings;

    private string _inputText = string.Empty;
    private int _baseDelayMs = 90;
    private int _jitterPercent = 20;
    private int _typoRatePercent = 8;
    private int _omissionRatePercent = 5;
    private int _transposeRatePercent = 4;
    private int _repairRatePercent = 85;
    private int _errorDetectDelayMs = 900;
    private int _backspaceDelayMs = 70;
    private int _leadInDelayMs = 2500;
    private string _statusText = "状态: Idle";
    private string _progressText = "进度: 0/0";
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

        _session.StateChanged += OnStateChanged;
        _session.ProgressChanged += OnProgressChanged;
        _session.LogProduced += OnLogProduced;

        _saveDebounceTimer = new DispatcherTimer
        {
            Interval = TimeSpan.FromMilliseconds(600)
        };
        _saveDebounceTimer.Tick += (_, _) => SaveSettingsNow();

        LoadSettings();

    }

    public event PropertyChangedEventHandler? PropertyChanged;

    public ICommand StartCommand => _startCommand;
    public ICommand PauseResumeCommand => _pauseResumeCommand;
    public ICommand ApplyHotkeysCommand => _applyHotkeysCommand;

    public event Action<string, string>? HotkeyUpdateRequested;

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

    public int BaseDelayMs
    {
        get => _baseDelayMs;
        set
        {
            if (SetField(ref _baseDelayMs, value))
            {
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
            System.Windows.MessageBox.Show("请先粘贴文本。", "humanize_input", System.Windows.MessageBoxButton.OK, System.Windows.MessageBoxImage.Information);
            return;
        }

        StatusText = $"状态: 准备开始，{LeadInDelayMs}ms 后抓取目标窗口";
        await Task.Delay(LeadInDelayMs);

        nint targetWindow = targetWindowOverride ?? _driver.GetForegroundWindowHandle();
        TypingSettings settings = BuildSettings();

        bool started = _session.Start(InputText, settings, targetWindow);
        if (!started)
        {
            StatusText = "状态: 无法开始（已有会话或输入为空）";
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
        StatusText = "状态: 已请求更新全局热键";
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
            StatusText = $"状态: {state}";
            RefreshCommands();
        });
    }

    private void OnProgressChanged(TypingProgress progress)
    {
        System.Windows.Application.Current.Dispatcher.Invoke(() =>
        {
            ProgressText = $"进度: {progress.TypedChars}/{progress.TotalChars} | 错字: {progress.TypoCount} | 漏写: {progress.OmissionCount} | 颠倒: {progress.TransposeCount} | 修正: {progress.CorrectionCount}";
        });
    }

    private void OnLogProduced(string message)
    {
        System.Windows.Application.Current.Dispatcher.Invoke(() =>
        {
            StatusText = $"状态: {message}";
        });
    }

    private void RefreshCommands()
    {
        _startCommand.RaiseCanExecuteChanged();
        _pauseResumeCommand.RaiseCanExecuteChanged();
    }

    private bool SetField<T>(ref T field, T value, [CallerMemberName] string? propertyName = null)
    {
        if (EqualityComparer<T>.Default.Equals(field, value))
        {
            return false;
        }

        field = value;
        PropertyChanged?.Invoke(this, new PropertyChangedEventArgs(propertyName));
        return true;
    }
}
