using HumanizeInput.Core.Models;
using HumanizeInput.Core.Timing;

namespace HumanizeInput.Core;

public sealed class TypingSessionService
{
    private static readonly string[] KeyboardRows =
    [
        "`1234567890-=",
        "qwertyuiop[]\\",
        "asdfghjkl;'",
        "zxcvbnm,./"
    ];

    private static readonly double[] RowOffsets = [0.0, 0.5, 1.0, 1.5];

    private static readonly Dictionary<char, (int row, int col)> KeyPositions = BuildKeyPositions();

    private static readonly Dictionary<char, char> ShiftedToBase = new()
    {
        ['~'] = '`', ['!'] = '1', ['@'] = '2', ['#'] = '3', ['$'] = '4', ['%'] = '5',
        ['^'] = '6', ['&'] = '7', ['*'] = '8', ['('] = '9', [')'] = '0', ['_'] = '-', ['+'] = '=',
        ['{'] = '[', ['}'] = ']', ['|'] = '\\', [':'] = ';', ['"'] = '\'', ['<'] = ',', ['>'] = '.', ['?'] = '/'
    };

    private readonly ITypingDriver _driver;
    private readonly ManualResetEventSlim _pauseGate = new(initialState: true);
    private readonly object _sync = new();
    private CancellationTokenSource? _cts;
    private Task? _runningTask;

    public TypingSessionService(ITypingDriver driver)
    {
        _driver = driver;
    }

    public SessionState State { get; private set; } = SessionState.Idle;

    public event Action<SessionState>? StateChanged;
    public event Action<TypingProgress>? ProgressChanged;
    public event Action<string>? LogProduced;

    public bool IsBusy => State is SessionState.Running or SessionState.Paused;

    public bool Start(string text, TypingSettings settings, nint targetWindow)
    {
        if (string.IsNullOrWhiteSpace(text))
        {
            LogProduced?.Invoke("输入文本为空，已忽略开始请求。");
            return false;
        }

        if (targetWindow == nint.Zero)
        {
            LogProduced?.Invoke("未检测到有效目标窗口，已取消开始。请先聚焦目标输入框再按开始热键。");
            return false;
        }

        lock (_sync)
        {
            if (IsBusy)
            {
                LogProduced?.Invoke("已有会话在运行或暂停中。");
                return false;
            }

            _cts = new CancellationTokenSource();
            _pauseGate.Set();
            SetState(SessionState.Running);

            _runningTask = Task.Run(() => RunSessionAsync(text, settings, targetWindow, _cts.Token));
            return true;
        }
    }

    public void Pause()
    {
        if (State != SessionState.Running)
        {
            return;
        }

        _pauseGate.Reset();
        SetState(SessionState.Paused);
        LogProduced?.Invoke("会话已暂停。");
    }

    public void Resume()
    {
        if (State != SessionState.Paused)
        {
            return;
        }

        _pauseGate.Set();
        SetState(SessionState.Running);
        LogProduced?.Invoke("会话已继续。");
    }

    public void Stop()
    {
        _cts?.Cancel();
        _pauseGate.Set();
        SetState(SessionState.Idle);
        LogProduced?.Invoke("会话已停止。");
    }

    private async Task RunSessionAsync(string text, TypingSettings settings, nint targetWindow, CancellationToken cancellationToken)
    {
        Random random = new();

        TypingProgress progress = new()
        {
            TotalChars = text.Length,
            TypedChars = 0,
            TypoCount = 0,
            OmissionCount = 0,
            TransposeCount = 0,
            CorrectionCount = 0
        };

        try
        {
            int index = 0;
            while (index < text.Length)
            {
                cancellationToken.ThrowIfCancellationRequested();
                _pauseGate.Wait(cancellationToken);

                await WaitForTargetFocusAsync(targetWindow, cancellationToken);

                char current = text[index];

                bool canTypo = !char.IsDigit(current);
                bool typo = canTypo && random.Next(0, 100) < settings.TypoRatePercent;
                bool omission = !typo && random.Next(0, 100) < settings.OmissionRatePercent;
                bool transpose = !typo
                                 && !omission
                                 && index + 1 < text.Length
                                 && !char.IsWhiteSpace(current)
                                 && !char.IsWhiteSpace(text[index + 1])
                                 && random.Next(0, 100) < settings.TransposeRatePercent;
                bool willRepair = random.Next(0, 100) < settings.RepairRatePercent;

                if (typo)
                {
                    char wrong = BuildWrongChar(current, random);
                    await _driver.TypeCharAsync(wrong, cancellationToken);

                    if (willRepair)
                    {
                        int lookaheadCount = GetLookaheadCount(random, text.Length - (index + 1));
                        if (lookaheadCount > 0)
                        {
                            await TypeForwardRangeAsync(text, index + 1, lookaheadCount, cancellationToken);
                        }

                        await Task.Delay(settings.ErrorDetectDelayMs, cancellationToken);
                        await BackspaceTimesAsync(lookaheadCount + 1, settings.BackspaceDelayMs, cancellationToken);
                        await _driver.TypeCharAsync(current, cancellationToken);

                        if (lookaheadCount > 0)
                        {
                            await TypeForwardRangeAsync(text, index + 1, lookaheadCount, cancellationToken);
                        }

                        int consumed = 1 + lookaheadCount;
                        progress = new TypingProgress
                        {
                            TotalChars = progress.TotalChars,
                            TypedChars = progress.TypedChars + consumed,
                            TypoCount = progress.TypoCount + 1,
                            OmissionCount = progress.OmissionCount,
                            TransposeCount = progress.TransposeCount,
                            CorrectionCount = progress.CorrectionCount + 1
                        };
                        index += consumed;
                    }
                    else
                    {
                        progress = new TypingProgress
                        {
                            TotalChars = progress.TotalChars,
                            TypedChars = progress.TypedChars + 1,
                            TypoCount = progress.TypoCount + 1,
                            OmissionCount = progress.OmissionCount,
                            TransposeCount = progress.TransposeCount,
                            CorrectionCount = progress.CorrectionCount
                        };
                        index++;
                    }
                }
                else if (omission)
                {
                    if (willRepair)
                    {
                        int lookaheadCount = GetLookaheadCount(random, text.Length - (index + 1));
                        if (lookaheadCount > 0)
                        {
                            await TypeForwardRangeAsync(text, index + 1, lookaheadCount, cancellationToken);
                        }

                        await Task.Delay(settings.ErrorDetectDelayMs, cancellationToken);
                        await BackspaceTimesAsync(lookaheadCount, settings.BackspaceDelayMs, cancellationToken);
                        await _driver.TypeCharAsync(current, cancellationToken);

                        if (lookaheadCount > 0)
                        {
                            await TypeForwardRangeAsync(text, index + 1, lookaheadCount, cancellationToken);
                        }

                        int consumed = 1 + lookaheadCount;
                        progress = new TypingProgress
                        {
                            TotalChars = progress.TotalChars,
                            TypedChars = progress.TypedChars + consumed,
                            TypoCount = progress.TypoCount,
                            OmissionCount = progress.OmissionCount + 1,
                            TransposeCount = progress.TransposeCount,
                            CorrectionCount = progress.CorrectionCount + 1
                        };
                        index += consumed;
                    }
                    else
                    {
                        progress = new TypingProgress
                        {
                            TotalChars = progress.TotalChars,
                            TypedChars = progress.TypedChars + 1,
                            TypoCount = progress.TypoCount,
                            OmissionCount = progress.OmissionCount + 1,
                            TransposeCount = progress.TransposeCount,
                            CorrectionCount = progress.CorrectionCount
                        };
                        index++;
                    }
                }
                else if (transpose)
                {
                    char next = text[index + 1];
                    await _driver.TypeCharAsync(next, cancellationToken);
                    await Task.Delay(Math.Max(10, settings.BackspaceDelayMs / 2), cancellationToken);
                    await _driver.TypeCharAsync(current, cancellationToken);

                    if (willRepair)
                    {
                        int lookaheadCount = GetLookaheadCount(random, text.Length - (index + 2));
                        if (lookaheadCount > 0)
                        {
                            await TypeForwardRangeAsync(text, index + 2, lookaheadCount, cancellationToken);
                        }

                        await Task.Delay(settings.ErrorDetectDelayMs, cancellationToken);
                        await BackspaceTimesAsync(lookaheadCount + 2, settings.BackspaceDelayMs, cancellationToken);
                        await _driver.TypeCharAsync(current, cancellationToken);
                        await Task.Delay(Math.Max(10, settings.BackspaceDelayMs / 2), cancellationToken);
                        await _driver.TypeCharAsync(next, cancellationToken);

                        if (lookaheadCount > 0)
                        {
                            await TypeForwardRangeAsync(text, index + 2, lookaheadCount, cancellationToken);
                        }

                        int consumed = 2 + lookaheadCount;
                        progress = new TypingProgress
                        {
                            TotalChars = progress.TotalChars,
                            TypedChars = progress.TypedChars + consumed,
                            TypoCount = progress.TypoCount,
                            OmissionCount = progress.OmissionCount,
                            TransposeCount = progress.TransposeCount + 1,
                            CorrectionCount = progress.CorrectionCount + 1
                        };
                        index += consumed;
                    }
                    else
                    {
                        progress = new TypingProgress
                        {
                            TotalChars = progress.TotalChars,
                            TypedChars = progress.TypedChars + 2,
                            TypoCount = progress.TypoCount,
                            OmissionCount = progress.OmissionCount,
                            TransposeCount = progress.TransposeCount + 1,
                            CorrectionCount = progress.CorrectionCount
                        };
                        index += 2;
                    }
                }
                else
                {
                    await _driver.TypeCharAsync(current, cancellationToken);

                    progress = new TypingProgress
                    {
                        TotalChars = progress.TotalChars,
                        TypedChars = progress.TypedChars + 1,
                        TypoCount = progress.TypoCount,
                        OmissionCount = progress.OmissionCount,
                        TransposeCount = progress.TransposeCount,
                        CorrectionCount = progress.CorrectionCount
                    };
                    index++;
                }

                ProgressChanged?.Invoke(progress);
                int delay = DelayModel.NextDelayMs(random, settings.BaseDelayMs, settings.JitterPercent);
                await Task.Delay(delay, cancellationToken);
            }

            SetState(SessionState.Completed);
            LogProduced?.Invoke("会话输入完成。");
        }
        catch (OperationCanceledException)
        {
            SetState(SessionState.Idle);
        }
        catch (Exception ex)
        {
            SetState(SessionState.Faulted);
            LogProduced?.Invoke($"输入失败: {ex.Message}");
        }
        finally
        {
            lock (_sync)
            {
                _cts?.Dispose();
                _cts = null;
                _runningTask = null;
            }

            if (State is not SessionState.Faulted and not SessionState.Completed)
            {
                SetState(SessionState.Idle);
            }
        }
    }

    private static char BuildWrongChar(char current, Random random)
    {
        if (TryPickNeighborKey(current, random, out char neighbor))
        {
            return neighbor;
        }

        if (!char.IsLetter(current))
        {
            return random.Next(0, 2) == 0 ? 'x' : 'z';
        }

        if (char.IsLower(current))
        {
            return (char)('a' + random.Next(0, 26));
        }

        return (char)('A' + random.Next(0, 26));
    }

    private static bool TryPickNeighborKey(char current, Random random, out char neighbor)
    {
        neighbor = current;

        char lookup = char.ToLowerInvariant(current);
        bool isUpper = char.IsUpper(current);

        if (ShiftedToBase.TryGetValue(current, out char baseCharFromShifted))
        {
            lookup = baseCharFromShifted;
            isUpper = false;
        }

        if (!KeyPositions.TryGetValue(lookup, out var origin))
        {
            return false;
        }

        List<(char key, double weight)> candidates = [];
        foreach (KeyValuePair<char, (int row, int col)> item in KeyPositions)
        {
            if (item.Key == lookup)
            {
                continue;
            }

            if (!char.IsDigit(lookup) && char.IsDigit(item.Key))
            {
                continue;
            }

            double dx = (item.Value.col + RowOffsets[item.Value.row]) - (origin.col + RowOffsets[origin.row]);
            double dy = item.Value.row - origin.row;
            double distance = Math.Sqrt((dx * dx) + (dy * dy));
            if (distance < 0.0001 || distance > 2.4)
            {
                continue;
            }

            // Distance-based decay: closer neighbors are chosen much more often.
            double weight = 1.0 / (1.0 + (distance * distance * 2.2));
            candidates.Add((item.Key, weight));
        }

        if (candidates.Count == 0)
        {
            return false;
        }

        double totalWeight = candidates.Sum(c => c.weight);
        double pick = random.NextDouble() * totalWeight;

        char selected = candidates[^1].key;
        foreach ((char key, double weight) in candidates)
        {
            pick -= weight;
            if (pick <= 0)
            {
                selected = key;
                break;
            }
        }

        neighbor = isUpper ? char.ToUpperInvariant(selected) : selected;
        return true;
    }

    private static int GetLookaheadCount(Random random, int remaining)
    {
        if (remaining <= 0)
        {
            return 0;
        }

        // Around one third of repairs happen after continuing to type 1-2 chars.
        if (random.NextDouble() >= 0.35)
        {
            return 0;
        }

        int candidate = random.Next(1, 3);
        return Math.Min(candidate, remaining);
    }

    private async Task TypeForwardRangeAsync(string text, int startIndex, int count, CancellationToken cancellationToken)
    {
        for (int i = 0; i < count; i++)
        {
            await _driver.TypeCharAsync(text[startIndex + i], cancellationToken);
        }
    }

    private async Task BackspaceTimesAsync(int count, int backspaceDelayMs, CancellationToken cancellationToken)
    {
        for (int i = 0; i < count; i++)
        {
            await _driver.BackspaceAsync(cancellationToken);
            await Task.Delay(backspaceDelayMs, cancellationToken);
        }
    }

    private static Dictionary<char, (int row, int col)> BuildKeyPositions()
    {
        Dictionary<char, (int row, int col)> map = new();
        for (int row = 0; row < KeyboardRows.Length; row++)
        {
            string line = KeyboardRows[row];
            for (int col = 0; col < line.Length; col++)
            {
                char key = line[col];
                if (!map.ContainsKey(key))
                {
                    map[key] = (row, col);
                }
            }
        }

        return map;
    }

    private async Task WaitForTargetFocusAsync(nint targetWindow, CancellationToken cancellationToken)
    {
        if (_driver.GetForegroundWindowHandle() == targetWindow)
        {
            return;
        }

        if (State == SessionState.Running)
        {
            SetState(SessionState.Paused);
            LogProduced?.Invoke("检测到目标窗口失焦，等待焦点恢复后自动继续。");
        }

        while (_driver.GetForegroundWindowHandle() != targetWindow)
        {
            cancellationToken.ThrowIfCancellationRequested();
            _pauseGate.Wait(cancellationToken);
            await Task.Delay(120, cancellationToken);
        }

        if (State == SessionState.Paused)
        {
            SetState(SessionState.Running);
            LogProduced?.Invoke("目标窗口焦点已恢复，继续输入。");
        }
    }

    private void SetState(SessionState newState)
    {
        State = newState;
        StateChanged?.Invoke(newState);
    }
}
