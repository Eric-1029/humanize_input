namespace HumanizeInput.App.Analysis;

internal sealed class TypingSampleAnalyzer
{
    private enum AlignmentOperation
    {
        Match,
        Transposition,
        Substitution,
        Deletion,
        Insertion
    }

    private readonly record struct AlignmentCounts(int Insertions, int Deletions, int Substitutions, int Transpositions)
    {
        public int Omissions => Deletions;
    }

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

        AlignmentCounts alignment = AnalyzeAlignment(_promptText, _typedText);
        int omissions = alignment.Omissions;
        int transpositions = alignment.Transpositions;
        int substitutions = alignment.Substitutions;
        int insertions = alignment.Insertions;
        int repairRate = _errorBlocks > 0
            ? Clamp((int)Math.Round((_correctedErrorBlocks * 100.0) / _errorBlocks), 0, 100)
            : 85;

        int typoRate = Clamp((int)Math.Round(((substitutions + insertions + Math.Max(0, _deletedCharacters - omissions)) * 100.0) / promptLength), 0, 30);
        int omissionRate = Clamp((int)Math.Round((omissions * 100.0) / promptLength), 0, 30);
        int transposeRate = Clamp((int)Math.Round((transpositions * 100.0) / promptLength), 0, 20);

        int errorDetectDelayMs = _correctionDelaysMs.Count > 0
            ? Clamp((int)Math.Round(_correctionDelaysMs.Average()), 80, 3000)
            : Clamp(baseDelayMs * 3, 80, 3000);

        int backspaceDelayMs = _backspaceIntervalsMs.Count > 0
            ? Clamp((int)Math.Round(_backspaceIntervalsMs.Average()), 10, 500)
            : Clamp((int)Math.Round(baseDelayMs * 0.8), 10, 500);

        double accuracyPercent = ClampDouble(100.0 - ((omissions + substitutions + insertions + transpositions) * 100.0 / promptLength), 0, 100);
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

    private static AlignmentCounts AnalyzeAlignment(string prompt, string typed)
    {
        int promptLength = prompt.Length;
        int typedLength = typed.Length;
        int[,] costs = new int[promptLength + 1, typedLength + 1];
        AlignmentOperation[,] operations = new AlignmentOperation[promptLength + 1, typedLength + 1];

        for (int i = 1; i <= promptLength; i++)
        {
            costs[i, 0] = i;
            operations[i, 0] = AlignmentOperation.Deletion;
        }

        for (int j = 1; j <= typedLength; j++)
        {
            costs[0, j] = j;
            operations[0, j] = AlignmentOperation.Insertion;
        }

        for (int i = 1; i <= promptLength; i++)
        {
            for (int j = 1; j <= typedLength; j++)
            {
                int bestCost = costs[i - 1, j] + 1;
                AlignmentOperation bestOperation = AlignmentOperation.Deletion;

                TryUpdateBest(costs[i, j - 1] + 1, AlignmentOperation.Insertion, ref bestCost, ref bestOperation);

                bool isMatch = prompt[i - 1] == typed[j - 1];
                TryUpdateBest(costs[i - 1, j - 1] + (isMatch ? 0 : 1), isMatch ? AlignmentOperation.Match : AlignmentOperation.Substitution, ref bestCost, ref bestOperation);

                if (i > 1 && j > 1 && prompt[i - 1] == typed[j - 2] && prompt[i - 2] == typed[j - 1])
                {
                    TryUpdateBest(costs[i - 2, j - 2] + 1, AlignmentOperation.Transposition, ref bestCost, ref bestOperation);
                }

                costs[i, j] = bestCost;
                operations[i, j] = bestOperation;
            }
        }

        int insertions = 0;
        int deletions = 0;
        int substitutions = 0;
        int transpositions = 0;

        int row = promptLength;
        int column = typedLength;
        while (row > 0 || column > 0)
        {
            if (row == 0)
            {
                insertions += column;
                break;
            }

            if (column == 0)
            {
                deletions += row;
                break;
            }

            switch (operations[row, column])
            {
                case AlignmentOperation.Match:
                    row--;
                    column--;
                    break;
                case AlignmentOperation.Substitution:
                    substitutions++;
                    row--;
                    column--;
                    break;
                case AlignmentOperation.Transposition:
                    transpositions++;
                    row -= 2;
                    column -= 2;
                    break;
                case AlignmentOperation.Deletion:
                    deletions++;
                    row--;
                    break;
                case AlignmentOperation.Insertion:
                    insertions++;
                    column--;
                    break;
            }
        }

        return new AlignmentCounts(insertions, deletions, substitutions, transpositions);
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

    private static void TryUpdateBest(int candidateCost, AlignmentOperation candidateOperation, ref int bestCost, ref AlignmentOperation bestOperation)
    {
        if (candidateCost < bestCost || (candidateCost == bestCost && OperationPriority(candidateOperation) < OperationPriority(bestOperation)))
        {
            bestCost = candidateCost;
            bestOperation = candidateOperation;
        }
    }

    private static int OperationPriority(AlignmentOperation operation)
    {
        return operation switch
        {
            AlignmentOperation.Match => 0,
            AlignmentOperation.Transposition => 1,
            AlignmentOperation.Substitution => 2,
            AlignmentOperation.Deletion => 3,
            AlignmentOperation.Insertion => 4,
            _ => 5
        };
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
