using System.ComponentModel;
using System.Runtime.CompilerServices;
using System.Windows.Input;
using System.Windows.Threading;
using HumanizeInput.App.Commands;

namespace HumanizeInput.App.Analysis;

public sealed class TypingFrequencyDetectorViewModel : INotifyPropertyChanged
{
    private const string LanguageZhCn = "zh-CN";
    private const string LanguageEnUs = "en-US";

    private readonly string _uiLanguageCode;
    private readonly TypingSampleAnalyzer _analyzer;
    private readonly RelayCommand _togglePromptLanguageCommand;
    private readonly RelayCommand _resetCommand;
    private readonly RelayCommand _generateFitCommand;
    private readonly RelayCommand _captureToggleCommand;
    private readonly DispatcherTimer _captureTimer;

    private string _promptLanguageCode;
    private string _promptText;
    private string _typedText = string.Empty;
    private string _statusText;
    private string _liveMetricsText;
    private string _resultSummaryText;
    private TypingFitResult? _latestResult;
    private bool _isRecording;
    private bool _suppressTextRecording;
    private int _capturedElapsedMilliseconds;

    public TypingFrequencyDetectorViewModel(string uiLanguageCode)
    {
        _uiLanguageCode = NormalizeLanguage(uiLanguageCode);
        _promptLanguageCode = _uiLanguageCode;
        _promptText = BuildPromptText(_promptLanguageCode);
        _analyzer = new TypingSampleAnalyzer(_promptText);
        _togglePromptLanguageCommand = new RelayCommand(TogglePromptLanguage);
        _resetCommand = new RelayCommand(ResetSample);
        _generateFitCommand = new RelayCommand(() => GenerateFit());
        _captureToggleCommand = new RelayCommand(ToggleCapture);
        _captureTimer = new DispatcherTimer
        {
            Interval = TimeSpan.FromMilliseconds(250)
        };
        _captureTimer.Tick += (_, _) => UpdateLiveMetricsText();
        _analyzer.Reset(_promptText);
        _statusText = Translate("点击开始后再输入样本，停止后会自动生成拟合结果。", "Click Start to record the sample, then Stop to generate the fit.");
        _liveMetricsText = string.Empty;
        _resultSummaryText = Translate("尚未生成拟合结果。", "No fitted result yet.");
        UpdateLiveMetricsText();
    }

    public event PropertyChangedEventHandler? PropertyChanged;

    public string WindowTitle => Translate("输入频率检测器", "Typing Frequency Detector");

    public string PromptSectionTitle => Translate("默认文本", "Sample Text");

    public string InputSectionTitle => Translate("输入区", "Typing Area");

    public string MetricsSectionTitle => Translate("实时指标", "Live Metrics");

    public string ResultSectionTitle => Translate("拟合结果", "Fitted Result");

    public string StatusText
    {
        get => _statusText;
        private set => SetField(ref _statusText, value);
    }

    public string PromptLanguageName => string.Equals(_promptLanguageCode, LanguageZhCn, StringComparison.OrdinalIgnoreCase)
        ? Translate("中文示例", "Chinese Sample")
        : Translate("英文示例", "English Sample");

    public string PromptLanguageToggleText => string.Equals(_promptLanguageCode, LanguageZhCn, StringComparison.OrdinalIgnoreCase)
        ? Translate("切换到英文示例", "Switch to English sample")
        : Translate("切换到中文示例", "Switch to Chinese sample");

    public string ResetButtonText => Translate("重新开始", "Reset");

    public string CaptureButtonText => _isRecording ? Translate("停止", "Stop") : Translate("开始", "Start");

    public string ApplyButtonText => Translate("应用到主设置", "Apply to Main Settings");

    public string CloseButtonText => Translate("关闭", "Close");

    public string PromptText
    {
        get => _promptText;
        private set => SetField(ref _promptText, value);
    }

    public string TypedText
    {
        get => _typedText;
        set
        {
            if (string.Equals(_typedText, value, StringComparison.Ordinal))
            {
                return;
            }

            string previousText = _typedText;
            _typedText = value;
            OnPropertyChanged(nameof(TypedText));

            if (_suppressTextRecording)
            {
                _suppressTextRecording = false;
            }
            else if (_isRecording)
            {
                _analyzer.RecordTextChange(previousText, value, DateTimeOffset.UtcNow);
                _latestResult = null;
                OnPropertyChanged(nameof(HasResult));
                OnPropertyChanged(nameof(CanApply));
                UpdateResultSummaryText();
                StatusText = Translate("已更新输入，继续打字或重新生成拟合参数。", "Input updated; keep typing or generate the fit again.");
            }

            UpdateLiveMetricsText();
        }
    }

    public string LiveMetricsText
    {
        get => _liveMetricsText;
        private set => SetField(ref _liveMetricsText, value);
    }

    public string ResultSummaryText
    {
        get => _resultSummaryText;
        private set => SetField(ref _resultSummaryText, value);
    }

    public bool HasResult => _latestResult is not null;

    public bool CanApply => !_isRecording && _latestResult is not null;

    public bool IsRecording => _isRecording;

    public TypingFitResult? LatestResult => _latestResult;

    public ICommand TogglePromptLanguageCommand => _togglePromptLanguageCommand;

    public ICommand ResetCommand => _resetCommand;

    public ICommand GenerateFitCommand => _generateFitCommand;

    public ICommand CaptureToggleCommand => _captureToggleCommand;

    public void ResetSample()
    {
        _suppressTextRecording = true;
        TypedText = string.Empty;
        _suppressTextRecording = false;
        _analyzer.Reset(_promptText);
        _latestResult = null;
        _capturedElapsedMilliseconds = 0;
        RaiseCaptureStateChanged();
        OnPropertyChanged(nameof(HasResult));
        OnPropertyChanged(nameof(CanApply));
        UpdateResultSummaryText();
        UpdateLiveMetricsText();
        StatusText = _isRecording
            ? Translate("样本已重置，继续输入。", "Sample reset; keep typing.")
            : Translate("样本已重置，点击开始后输入。", "Sample reset; click Start to begin.");
    }

    public void ToggleCapture()
    {
        if (_isRecording)
        {
            StopCapture();
            return;
        }

        StartCapture();
    }

    public bool GenerateFit()
    {
        if (TypedText.Length == 0)
        {
            StatusText = Translate("请先输入一些内容，再生成拟合参数。", "Please type some text before generating a fit.");
            _latestResult = null;
            OnPropertyChanged(nameof(HasResult));
            OnPropertyChanged(nameof(CanApply));
            UpdateResultSummaryText();
            return false;
        }

        _latestResult = _analyzer.Analyze(_promptLanguageCode, _uiLanguageCode);
        OnPropertyChanged(nameof(HasResult));
        OnPropertyChanged(nameof(CanApply));
        UpdateResultSummaryText();
        StatusText = Translate("已生成拟合参数，可以应用到主设置。", "Fit generated; you can apply it to the main settings.");
        return true;
    }

    private void TogglePromptLanguage()
    {
        _promptLanguageCode = string.Equals(_promptLanguageCode, LanguageZhCn, StringComparison.OrdinalIgnoreCase)
            ? LanguageEnUs
            : LanguageZhCn;

        PromptText = BuildPromptText(_promptLanguageCode);
        _analyzer.Reset(_promptText);
        _suppressTextRecording = true;
        TypedText = string.Empty;
        _suppressTextRecording = false;
        _latestResult = null;
        _capturedElapsedMilliseconds = 0;
        RaiseCaptureStateChanged();
        OnPropertyChanged(nameof(PromptLanguageName));
        OnPropertyChanged(nameof(PromptLanguageToggleText));
        OnPropertyChanged(nameof(HasResult));
        OnPropertyChanged(nameof(CanApply));
        UpdateResultSummaryText();
        UpdateLiveMetricsText();
        StatusText = _isRecording
            ? Translate("示例文本已切换，继续输入。", "Sample text switched; keep typing.")
            : Translate("示例文本已切换，点击开始后输入。", "Sample text switched; click Start to begin.");
    }

    private void UpdateLiveMetricsText()
    {
        double averageTypingInterval = _analyzer.AverageInsertIntervalMs;
        double averageBackspaceInterval = _analyzer.AverageBackspaceIntervalMs;
        double averageCorrectionDelay = _analyzer.AverageCorrectionDelayMs;
        int elapsedMilliseconds = _isRecording ? _analyzer.ElapsedMilliseconds : _capturedElapsedMilliseconds;
        int elapsedSeconds = (int)Math.Floor(elapsedMilliseconds / 1000.0);

        if (string.Equals(_uiLanguageCode, LanguageZhCn, StringComparison.OrdinalIgnoreCase))
        {
            LiveMetricsText =
                $"已用时: {elapsedSeconds}s\n" +
                $"示例长度: {_analyzer.PromptLength}\n" +
                $"已输入: {_analyzer.TypedLength}\n" +
                $"新增字符: {_analyzer.InsertedCharacters}\n" +
                $"删除字符: {_analyzer.DeletedCharacters}\n" +
                $"退格次数: {_analyzer.BackspaceEvents}\n" +
                $"平均键间隔: {(averageTypingInterval > 0 ? $"{averageTypingInterval:F0} ms" : "--")}\n" +
                $"平均退格间隔: {(averageBackspaceInterval > 0 ? $"{averageBackspaceInterval:F0} ms" : "--")}\n" +
                $"误差块: {_analyzer.ErrorBlocks}\n" +
                $"已修正: {_analyzer.CorrectedErrorBlocks}\n" +
                $"平均修正延迟: {(averageCorrectionDelay > 0 ? $"{averageCorrectionDelay:F0} ms" : "--")}";
            return;
        }

        LiveMetricsText =
            $"Elapsed: {elapsedSeconds}s\n" +
            $"Prompt length: {_analyzer.PromptLength}\n" +
            $"Typed: {_analyzer.TypedLength}\n" +
            $"Inserted chars: {_analyzer.InsertedCharacters}\n" +
            $"Deleted chars: {_analyzer.DeletedCharacters}\n" +
            $"Backspaces: {_analyzer.BackspaceEvents}\n" +
            $"Avg key interval: {(averageTypingInterval > 0 ? $"{averageTypingInterval:F0} ms" : "--")}\n" +
            $"Avg backspace interval: {(averageBackspaceInterval > 0 ? $"{averageBackspaceInterval:F0} ms" : "--")}\n" +
            $"Error blocks: {_analyzer.ErrorBlocks}\n" +
            $"Corrected: {_analyzer.CorrectedErrorBlocks}\n" +
            $"Avg correction delay: {(averageCorrectionDelay > 0 ? $"{averageCorrectionDelay:F0} ms" : "--")}";
    }

    private void UpdateResultSummaryText()
    {
        ResultSummaryText = _latestResult?.Summary ?? Translate("尚未生成拟合结果。", "No fitted result yet.");
    }

    private void StartCapture()
    {
        _analyzer.Reset(_promptText);
        _latestResult = null;
        _capturedElapsedMilliseconds = 0;
        _suppressTextRecording = true;
        TypedText = string.Empty;
        _suppressTextRecording = false;
        _isRecording = true;
        _captureTimer.Start();

        RaiseCaptureStateChanged();
        UpdateResultSummaryText();
        UpdateLiveMetricsText();
        StatusText = Translate("已开始计时，请输入样本。", "Recording started; type the sample now.");
    }

    private void StopCapture()
    {
        if (!_isRecording)
        {
            return;
        }

        _isRecording = false;
        _captureTimer.Stop();
        _capturedElapsedMilliseconds = _analyzer.ElapsedMilliseconds;
        RaiseCaptureStateChanged();

        bool generated = GenerateFit();
        StatusText = generated
            ? Translate("已停止计时，并生成拟合结果。", "Recording stopped and the fit was generated.")
            : Translate("已停止计时，但样本为空。", "Recording stopped, but the sample is empty.");
        UpdateLiveMetricsText();
    }

    private string BuildPromptText(string languageCode)
    {
        if (string.Equals(languageCode, LanguageEnUs, StringComparison.OrdinalIgnoreCase))
        {
            return "Please type the sample below in the way you naturally type. We will fit typing speed, jitter, typo rate, omission rate, transpositions, and correction delay from your input.";
        }

        return "请在下方按你平常的输入方式完整打出这段文本。系统会根据你的输入速度、抖动、错字、漏写、颠倒和修正停顿来拟合参数。";
    }

    private string Translate(string zh, string en)
    {
        return string.Equals(_uiLanguageCode, LanguageZhCn, StringComparison.OrdinalIgnoreCase) ? zh : en;
    }

    private static string NormalizeLanguage(string? language)
    {
        if (string.IsNullOrWhiteSpace(language))
        {
            return LanguageZhCn;
        }

        return language.StartsWith("en", StringComparison.OrdinalIgnoreCase) ? LanguageEnUs : LanguageZhCn;
    }

    private void OnPropertyChanged(string propertyName)
    {
        PropertyChanged?.Invoke(this, new PropertyChangedEventArgs(propertyName));
    }

    private void RaiseCaptureStateChanged()
    {
        OnPropertyChanged(nameof(CaptureButtonText));
        OnPropertyChanged(nameof(IsRecording));
        OnPropertyChanged(nameof(CanApply));
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
