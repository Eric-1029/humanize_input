namespace HumanizeInput.App.Analysis;

internal sealed class TypingSampleAnalyzer
{
    private readonly List<double> _insertIntervalsMs = new();
    private readonly List<double> _backspaceIntervalsMs = new();
    private readonly List<double> _correctionDelaysMs = new();

    private string _promptText;
    private string _typedText = string.Empty;
    private DateTimeOffset _sampleStartedAt;
    private DateTimeOffset? _firstInputAt;
    private DateTimeOffset? _lastInsertAt;
    private DateTimeOffset? _lastBackspaceAt;
    private DateTimeOffset? _errorStartedAt;
    private DateTimeOffset? _firstCorrectionAt;
    private int _insertedCharacters;
    private int _deletedCharacters;
    private int _backspaceEvents;
    private int _errorBlocks;
    private int _correctedErrorBlocks;

    public TypingSampleAnalyzer(string promptText)
    {
        _promptText = promptText;
        _sampleStartedAt = DateTimeOffset.UtcNow;
    }

    public string PromptText => _promptText;

    public string TypedText => _typedText;

    public int InsertedCharacters => _insertedCharacters;

    public int DeletedCharacters => _deletedCharacters;

    public int BackspaceEvents => _backspaceEvents;

    public int ErrorBlocks => _errorBlocks;

    public int CorrectedErrorBlocks => _correctedErrorBlocks;

    public int ElapsedMilliseconds => (int)Math.Max(0, (DateTimeOffset.UtcNow - _sampleStartedAt).TotalMilliseconds);

    public double AverageInsertIntervalMs => _insertIntervalsMs.Count > 0 ? _insertIntervalsMs.Average() : 0;

    public double AverageBackspaceIntervalMs => _backspaceIntervalsMs.Count > 0 ? _backspaceIntervalsMs.Average() : 0;

    public double AverageCorrectionDelayMs => _correctionDelaysMs.Count > 0 ? _correctionDelaysMs.Average() : 0;

    public int PromptLength => _promptText.Length;

    public int TypedLength => _typedText.Length;

    public void Reset(string promptText)
    {
        _promptText = promptText;
        _typedText = string.Empty;
        _sampleStartedAt = DateTimeOffset.UtcNow;
        _firstInputAt = null;
        _lastInsertAt = null;
        _lastBackspaceAt = null;
        _errorStartedAt = null;
        _firstCorrectionAt = null;
        _insertedCharacters = 0;
        _deletedCharacters = 0;
        _backspaceEvents = 0;
        _errorBlocks = 0;
        _correctedErrorBlocks = 0;
        _insertIntervalsMs.Clear();
        _backspaceIntervalsMs.Clear();
        _correctionDelaysMs.Clear();
    }

    public void RecordTextChange(string previousText, string currentText, DateTimeOffset timestamp)
    {
        if (currentText == previousText)
        {
            return;
        }

        int prefixLength = CommonPrefixLength(previousText, currentText);
        int suffixLength = CommonSuffixLength(previousText, currentText, prefixLength);

        string deletedSegment = previousText[prefixLength..(previousText.Length - suffixLength)];
        string insertedSegment = currentText[prefixLength..(currentText.Length - suffixLength)];

        if (insertedSegment.Length > 0)
        {
            if (_firstInputAt is null)
            {
                _firstInputAt = timestamp;
            }

            if (insertedSegment.Length == 1 && _lastInsertAt is not null)
            {
                double interval = (timestamp - _lastInsertAt.Value).TotalMilliseconds;
                if (interval >= 0 && interval <= 5000)
                {
                    _insertIntervalsMs.Add(interval);
                }
            }

            if (insertedSegment.Length == 1)
            {
                _lastInsertAt = timestamp;
            }
            else
            {
                _lastInsertAt = null;
            }

            _insertedCharacters += insertedSegment.Length;
        }

        if (deletedSegment.Length > 0)
        {
            _backspaceEvents += deletedSegment.Length;
            _deletedCharacters += deletedSegment.Length;

            if (_lastBackspaceAt is not null)
            {
                double interval = (timestamp - _lastBackspaceAt.Value).TotalMilliseconds;
                if (interval >= 0 && interval <= 5000)
                {
                    _backspaceIntervalsMs.Add(interval);
                }
            }

            _lastBackspaceAt = timestamp;

            if (_errorStartedAt is not null && _firstCorrectionAt is null)
            {
                _firstCorrectionAt = timestamp;
            }
        }

        _typedText = currentText;
        UpdateErrorState(timestamp);
    }

    public TypingFitResult Analyze(string promptLanguageCode, string uiLanguageCode)
    {
        int promptLength = Math.Max(1, _promptText.Length);
        int typedLength = _typedText.Length;
        int totalElapsedMs = ElapsedMilliseconds;
        int leadInDelayMs = _firstInputAt is null
            ? 2500
            : Clamp((int)(_firstInputAt.Value - _sampleStartedAt).TotalMilliseconds, 0, 6000);

        double averageInterval = _insertIntervalsMs.Count > 0 ? _insertIntervalsMs.Average() : 90;
        double variance = _insertIntervalsMs.Count > 0 ? _insertIntervalsMs.Select(v => Math.Pow(v - averageInterval, 2)).Average() : 0;
        double standardDeviation = Math.Sqrt(variance);

        int baseDelayMs = Clamp((int)Math.Round(averageInterval), 20, 500);
        int jitterPercent = averageInterval > 0
            ? Clamp((int)Math.Round((standardDeviation / averageInterval) * 100.0), 0, 80)
            : 20;

        int omissions = CountOmissions(_promptText, _typedText);
        int transpositions = CountAdjacentTranspositions(_promptText, _typedText);
        int substitutions = CountSubstitutions(_promptText, _typedText, transpositions);
        int repairRate = _errorBlocks > 0
            ? Clamp((int)Math.Round((_correctedErrorBlocks * 100.0) / _errorBlocks), 0, 100)
            : 85;

        int typoRate = Clamp((int)Math.Round(((substitutions + Math.Max(0, _deletedCharacters - omissions)) * 100.0) / promptLength), 0, 30);
        int omissionRate = Clamp((int)Math.Round((omissions * 100.0) / promptLength), 0, 30);
        int transposeRate = Clamp((int)Math.Round((transpositions * 100.0) / promptLength), 0, 20);

        int errorDetectDelayMs = _correctionDelaysMs.Count > 0
            ? Clamp((int)Math.Round(_correctionDelaysMs.Average()), 80, 3000)
            : Clamp(baseDelayMs * 3, 80, 3000);

        int backspaceDelayMs = _backspaceIntervalsMs.Count > 0
            ? Clamp((int)Math.Round(_backspaceIntervalsMs.Average()), 10, 500)
            : Clamp((int)Math.Round(baseDelayMs * 0.8), 10, 500);

        double accuracyPercent = ClampDouble(100.0 - ((omissions + substitutions + transpositions) * 100.0 / promptLength), 0, 100);
        string summary = BuildSummary(promptLanguageCode, uiLanguageCode, totalElapsedMs, baseDelayMs, jitterPercent, typoRate, omissionRate, transposeRate, repairRate, errorDetectDelayMs, backspaceDelayMs, accuracyPercent);

        return new TypingFitResult
        {
            PromptLanguageCode = promptLanguageCode,
            BaseDelayMs = baseDelayMs,
            JitterPercent = jitterPercent,
            TypoRatePercent = typoRate,
            OmissionRatePercent = omissionRate,
            TransposeRatePercent = transposeRate,
            RepairRatePercent = repairRate,
            ErrorDetectDelayMs = errorDetectDelayMs,
            BackspaceDelayMs = backspaceDelayMs,
            LeadInDelayMs = leadInDelayMs,
            PromptLength = promptLength,
            TypedLength = typedLength,
            AccuracyPercent = accuracyPercent,
            Summary = summary
        };
    }

    private void UpdateErrorState(DateTimeOffset timestamp)
    {
        bool matchesPromptPrefix = IsPromptPrefixMatch(_typedText);

        if (!matchesPromptPrefix)
        {
            if (_errorStartedAt is null)
            {
                _errorStartedAt = timestamp;
                _firstCorrectionAt = null;
                _errorBlocks++;
            }

            return;
        }

        if (_errorStartedAt is null)
        {
            return;
        }

        if (_firstCorrectionAt is not null)
        {
            double correctionDelay = (_firstCorrectionAt.Value - _errorStartedAt.Value).TotalMilliseconds;
            if (correctionDelay >= 0 && correctionDelay <= 30000)
            {
                _correctionDelaysMs.Add(correctionDelay);
            }

            _correctedErrorBlocks++;
        }

        _errorStartedAt = null;
        _firstCorrectionAt = null;
    }

    private bool IsPromptPrefixMatch(string text)
    {
        if (text.Length > _promptText.Length)
        {
            return false;
        }

        return _promptText.AsSpan(0, text.Length).SequenceEqual(text.AsSpan());
    }

    private static int CommonPrefixLength(string left, string right)
    {
        int max = Math.Min(left.Length, right.Length);
        int index = 0;
        while (index < max && left[index] == right[index])
        {
            index++;
        }

        return index;
    }

    private static int CommonSuffixLength(string left, string right, int prefixLength)
    {
        int leftIndex = left.Length - 1;
        int rightIndex = right.Length - 1;
        int suffixLength = 0;

        while (leftIndex >= prefixLength && rightIndex >= prefixLength && left[leftIndex] == right[rightIndex])
        {
            suffixLength++;
            leftIndex--;
            rightIndex--;
        }

        return suffixLength;
    }

    private static int CountOmissions(string prompt, string typed)
    {
        int count = 0;
        int limit = Math.Min(prompt.Length, typed.Length);
        for (int i = 0; i < limit; i++)
        {
            if (prompt[i] != typed[i])
            {
                count++;
            }
        }

        if (prompt.Length > typed.Length)
        {
            count += prompt.Length - typed.Length;
        }

        return count;
    }

    private static int CountSubstitutions(string prompt, string typed, int transpositions)
    {
        int limit = Math.Min(prompt.Length, typed.Length);
        int count = 0;
        for (int i = 0; i < limit; i++)
        {
            if (prompt[i] != typed[i])
            {
                count++;
            }
        }

        return Math.Max(0, count - transpositions * 2);
    }

    private static int CountAdjacentTranspositions(string prompt, string typed)
    {
        int count = 0;
        int i = 0;
        int j = 0;

        while (i < prompt.Length && j < typed.Length)
        {
            if (prompt[i] == typed[j])
            {
                i++;
                j++;
                continue;
            }

            if (i + 1 < prompt.Length && j + 1 < typed.Length && prompt[i] == typed[j + 1] && prompt[i + 1] == typed[j])
            {
                count++;
                i += 2;
                j += 2;
                continue;
            }

            i++;
            j++;
        }

        return count;
    }

    private static string BuildSummary(string promptLanguageCode, string uiLanguageCode, int totalElapsedMs, int baseDelayMs, int jitterPercent, int typoRate, int omissionRate, int transposeRate, int repairRate, int errorDetectDelayMs, int backspaceDelayMs, double accuracyPercent)
    {
        bool isChinese = string.Equals(uiLanguageCode, "zh-CN", StringComparison.OrdinalIgnoreCase);
        string languageName = string.Equals(promptLanguageCode, "en-US", StringComparison.OrdinalIgnoreCase) ? (isChinese ? "英文" : "English") : (isChinese ? "中文" : "Chinese");

        if (isChinese)
        {
            return $"样本语言: {languageName}\n" +
                   $"耗时: {totalElapsedMs} ms\n" +
                   $"准确率: {accuracyPercent:F1}%\n" +
                   $"基础延迟: {baseDelayMs} ms\n" +
                   $"抖动: {jitterPercent}%\n" +
                   $"错字率: {typoRate}%\n" +
                   $"漏写率: {omissionRate}%\n" +
                   $"颠倒率: {transposeRate}%\n" +
                   $"修复率: {repairRate}%\n" +
                   $"错误发现延迟: {errorDetectDelayMs} ms\n" +
                   $"回删延迟: {backspaceDelayMs} ms";
        }

        return $"Sample language: {languageName}\n" +
               $"Elapsed: {totalElapsedMs} ms\n" +
               $"Accuracy: {accuracyPercent:F1}%\n" +
               $"Base delay: {baseDelayMs} ms\n" +
               $"Jitter: {jitterPercent}%\n" +
               $"Typo rate: {typoRate}%\n" +
               $"Omission rate: {omissionRate}%\n" +
               $"Transpose rate: {transposeRate}%\n" +
               $"Repair rate: {repairRate}%\n" +
               $"Error detect delay: {errorDetectDelayMs} ms\n" +
               $"Backspace delay: {backspaceDelayMs} ms";
    }

    private static int Clamp(int value, int min, int max)
    {
        return Math.Min(Math.Max(value, min), max);
    }

    private static double ClampDouble(double value, double min, double max)
    {
        return Math.Min(Math.Max(value, min), max);
    }
}
